use crate::provider::SecretsProvider;
use crate::secrets::{MemSize, Secret, SecretError};
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
}

#[derive(Clone)]
pub struct EnvManager {
    secrets: Vec<Secret>,
    provider: Arc<dyn SecretsProvider>,
}

impl EnvManager {
    pub fn new(secrets: Vec<Secret>, provider: Arc<dyn SecretsProvider>) -> Self {
        Self { secrets, provider }
    }

    /// Returns a list of all file paths tracked by this manager.
    /// Used by the filesystem watcher to register watches.
    pub fn files(&self) -> Vec<PathBuf> {
        self.secrets
            .iter()
            .filter_map(|s| s.source().path().map(PathBuf::from))
            .collect()
    }

    /// Checks if a specific path is tracked by this manager.
    pub fn tracks(&self, path: &Path) -> bool {
        self.files().iter().any(|p| p == path)
    }

    /// Resolves the current environment state.
    pub async fn resolve(&self) -> Result<HashMap<String, SecretString>, EnvError> {
        let mut map = HashMap::new();

        for secret in &self.secrets {
            let content = secret.source().read().fetch()?;

            let content = match content {
                Some(c) => c,
                None => continue,
            };

            match secret {
                Secret::Anonymous(_) => {
                    let cursor = std::io::Cursor::new(content.as_bytes());
                    for item in dotenvy::from_read_iter(cursor) {
                        let (k, v) = item.map_err(|e| EnvError::Parse(e.to_string()))?;
                        map.insert(k, v);
                    }
                }
                Secret::Named { key, .. } => {
                    map.insert(key.clone(), content.into_owned());
                }
            }
        }
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
