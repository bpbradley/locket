//! Secret environment variable management, handling injection and resolution.
//!
//! This module bridges the gap between raw environment definitions (from `.env` files
//! or system env vars) and the `SecretsProvider`. It parses, detects secret references,
//! fetches them, and constructs a `HashMap` translating references to boxed SecretStrings,
//! which can be exposed by the caller for process injection.

use crate::provider::SecretsProvider;
use crate::secrets::{Secret, SecretError};
use crate::template::Template;
use secrecy::{ExposeSecret, SecretString};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tracing::info;

#[derive(Debug, thiserror::Error)]
pub enum EnvError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("secret error: {0}")]
    Secret(#[from] SecretError),

    #[error("provider error: {0}")]
    Provider(#[from] crate::provider::ProviderError),

    #[error("dotenv parse error: {0}")]
    Parse(String),

    #[error("task join error: {0}")]
    Join(#[from] tokio::task::JoinError),
}

#[derive(Clone)]
pub struct EnvManager {
    secrets: Vec<Secret>,
    provider: Arc<dyn SecretsProvider>,
}

/// Manages the resolution of secrets for process environments.
///
/// `EnvManager` is responsible for:
/// 1. Reading source files (like `.env` files).
/// 2. Parsing key-value pairs.
/// 3. Detecting secret references (e.g., `op://...`) within those values.
/// 4. Batch fetching the secrets from the provider.
/// 5. Returning a fully resolved map of environment variables safe for injection.
impl EnvManager {
    /// Create a new manager for a specific set of secret sources.
    pub fn new(secrets: Vec<Secret>, provider: Arc<dyn SecretsProvider>) -> Self {
        Self { secrets, provider }
    }

    /// Returns a list of all file paths tracked by this manager.
    ///
    /// This is primarily used by the filesystem watcher to register watches
    /// on `.env` files, ensuring the process environment can be updated if the source changes.
    /// This will return all paths that were registered with the manager, even if they no longer exist.
    pub fn files(&self) -> Vec<PathBuf> {
        self.secrets
            .iter()
            .filter_map(|s| s.source().path().map(|p| p.to_path_buf()))
            .collect()
    }

    /// Checks if a specific path is tracked by this manager.
    pub fn tracks(&self, path: &Path) -> bool {
        self.files().iter().any(|p| p == path)
    }

    /// Resolves the current environment state into a map of secure strings.
    ///
    /// This method performs I/O to read files and network requests to fetch secrets.
    /// This is done in two passes on the secret sources:
    /// 1. Reads all sources to build a map of raw values.
    /// 2. Scans raw values for templates, batches distinct secret keys,
    ///    and resolves them via the provider.
    ///
    /// The resolved content is returned as a map of `{ key -> SecretString }`.
    ///
    /// # Errors
    /// Returns `EnvError` if file reading fails, parsing fails, or the provider encounters an error.
    pub async fn resolve(&self) -> Result<HashMap<String, SecretString>, EnvError> {
        let secrets = self.secrets.clone();
        let map = tokio::task::spawn_blocking(move || {
            let mut inner = HashMap::new();

            for secret in secrets {
                let content = secret.source().read().fetch()?;

                let content = match content {
                    Some(c) => c,
                    None => continue,
                };

                match &secret {
                    Secret::Anonymous(_) => {
                        let cursor = std::io::Cursor::new(content.as_bytes());
                        for item in dotenvy::from_read_iter(cursor) {
                            let (k, v) = item.map_err(|e| EnvError::Parse(e.to_string()))?;
                            inner.insert(k, v);
                        }
                    }
                    Secret::Named { key, .. } => {
                        inner.insert(key.clone(), content.into_owned());
                    }
                }
            }
            Ok::<HashMap<String, String>, EnvError>(inner)
        })
        .await??;
        let mut references = Vec::new();

        for v in map.values() {
            let tpl = Template::new(v);
            let keys = tpl.keys();

            if !keys.is_empty() {
                for key in keys {
                    if self.provider.accepts_key(key) {
                        references.push(key.to_string());
                    }
                }
            } else if self.provider.accepts_key(v.trim()) {
                references.push(v.trim().to_string());
            }
        }

        if references.is_empty() {
            return Ok(wrap_all(map));
        }

        // Deduplicate to save provider calls
        references.sort();
        references.dedup();
        let ref_strs: Vec<&str> = references.iter().map(|s| s.as_str()).collect();

        info!(count = references.len(), "batch fetching secrets");
        let secrets_map = self.provider.fetch_map(&ref_strs).await?;

        let mut result = HashMap::with_capacity(map.len());

        for (k, v) in map {
            let tpl = Template::new(&v);

            if tpl.has_tags() {
                // Render string with multiple replacements
                let rendered =
                    tpl.render_with(|key| secrets_map.get(key).map(|s| s.expose_secret()));
                result.insert(k, SecretString::new(rendered.into_owned().into()));
            } else if self.provider.accepts_key(v.trim()) {
                // Direct replacement
                if let Some(secret_val) = secrets_map.get(v.trim()) {
                    result.insert(k, secret_val.clone());
                } else {
                    // Provider didn't find it, keep original
                    result.insert(k, SecretString::new(v.into()));
                }
            } else {
                // No secret reference found
                result.insert(k, SecretString::new(v.into()));
            }
        }
        Ok(result)
    }
}

fn wrap_all(map: HashMap<String, String>) -> HashMap<String, SecretString> {
    map.into_iter()
        .map(|(k, v)| (k, SecretString::new(v.into())))
        .collect()
}
