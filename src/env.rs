use crate::provider::SecretsProvider;
use crate::secrets::PathExt;
use crate::template::Template;
use secrecy::{ExposeSecret, SecretString};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;
use tracing::info;

#[derive(Debug, thiserror::Error)]
pub enum EnvError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("failed to parse .env file {path:?}: {source}")]
    DotEnv { path: PathBuf, #[source] source: dotenvy::Error },
    #[error("invalid env format '{0}'; expected KEY=VALUE or File Path")]
    InvalidFormat(String),
    #[error("provider error: {0}")]
    Provider(#[from] crate::provider::ProviderError),
    #[error("blocking task failed: {0}")]
    Join(#[from] tokio::task::JoinError),
    #[error("secret error: {0}")]
    SecretError(#[from] crate::secrets::SecretError),
}

// --- Sources ---
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvFile {
    path: PathBuf,
    values: HashMap<String, String>,
}

impl EnvFile {
    pub fn try_new(path: impl AsRef<Path>) -> Result<Self, EnvError> {
        let path = path.as_ref().canon()?;
        let mut f = Self{ path, values: HashMap::new() };
        f.load()?;
        Ok(f)
    }
    pub fn clear(&mut self) {
        self.values.clear();
    }
    pub fn load(&mut self) -> Result<(), EnvError> {
        if !self.path.exists() {
            self.clear();
            return Ok(());
        }
        let values = dotenvy::from_path_iter(&self.path)
            .map_err(|e| EnvError::DotEnv { path: self.path.to_path_buf(), source: e })?
            .collect::<Result<HashMap<String, String>, _>>()
            .map_err(|e| EnvError::DotEnv { path: self.path.to_path_buf(), source: e })?;
        self.values = values;
        Ok(())
    }
}

impl FromStr for EnvFile {
    type Err = EnvError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        EnvFile::try_new(s)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvVal {
    pub key: String,
    pub value: String,
}

#[derive(Debug, Clone)]
pub enum EnvSource {
    File(EnvFile),
    Val(EnvVal),
}

impl EnvSource {
    pub fn apply(&self, map: &mut HashMap<String, String>) {
        match self {
            EnvSource::File(f) => map.extend(f.values.clone()),
            EnvSource::Val(v) => { map.insert(v.key.clone(), v.value.clone()); },
        }
    }

    pub fn path(&self) -> Option<&Path> {
        match self {
            EnvSource::File(f) => Some(&f.path),
            EnvSource::Val(_) => None,
        }
    }
}

impl FromStr for EnvSource {
    type Err = EnvError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if let Some(path) = s.strip_prefix('@') {
            Ok(EnvSource::File(EnvFile::try_new(path)?))
        } else if s.contains('=') {
            let (key, value) = s.split_once('=').ok_or_else(|| EnvError::InvalidFormat(s.to_string()))?;
            Ok(EnvSource::Val(EnvVal { key: key.to_string(), value: value.to_string() }))
        } else {
            Ok(EnvSource::File(EnvFile::try_new(s)?))
        }
    }
}

#[derive(Clone)]
pub struct EnvManager {
    sources: Vec<EnvSource>,
    provider: Arc<dyn SecretsProvider>,
}

impl std::fmt::Debug for EnvManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EnvManager")
            .field("sources", &self.sources)
            .finish()
    }
}

impl EnvManager {
    pub fn new(sources: Vec<EnvSource>, provider: Arc<dyn SecretsProvider>) -> Self {
        Self { sources, provider }
    }

    pub fn files(&self) -> Vec<PathBuf> {
        self.sources.iter()
            .filter_map(|s| s.path().map(PathBuf::from))
            .collect()
    }

    pub fn tracks(&self, path: &Path) -> bool {
        self.files().iter().any(|p| p == path)
    }

    pub fn remove(&mut self, path: &Path) {
        for source in &mut self.sources {
            if let EnvSource::File(f) = source {
                if f.path == path {
                    f.clear();
                }
            }
        }
    }

    pub async fn reload(&mut self, path: &Path) -> Result<(), EnvError> {
        for source in &mut self.sources {
            if let EnvSource::File(f) = source {
                if f.path == path {
                    let mut f_clone = f.clone();
                    let updated_f = tokio::task::spawn_blocking(move || {
                        f_clone.load()?;
                        Ok::<_, EnvError>(f_clone)
                    }).await??;
                    *f = updated_f;
                }
            }
        }
        Ok(())
    }

    /// Resolves the current environment state.
    pub async fn resolve(&self) -> Result<HashMap<String, SecretString>, EnvError> {
        let mut map = HashMap::new();
        for source in &self.sources {
            source.apply(&mut map);
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

        references.sort();
        references.dedup();
        let ref_strs: Vec<&str> = references.iter().map(|s| s.as_str()).collect();

        info!(count = references.len(), "batch fetching secrets");
        let secrets_map = self.provider.fetch_map(&ref_strs).await?;

        // Pass 2: Rendering
        let mut final_map = HashMap::with_capacity(map.len());

        for (k, v) in map {
            let tpl = Template::new(&v);
            if tpl.has_tags() {
                let rendered = tpl.render_with(|key| {
                    secrets_map.get(key).map(|s| s.expose_secret())
                });
                final_map.insert(k, SecretString::new(rendered.into_owned().into()));
            } else if self.provider.accepts_key(v.trim()) {
                 if let Some(secret_val) = secrets_map.get(v.trim()) {
                    final_map.insert(k, secret_val.clone());
                } else {
                    final_map.insert(k, SecretString::new(v.into()));
                }
            } else {
                final_map.insert(k, SecretString::new(v.into()));
            }
        }
        Ok(final_map)
    }
}

fn wrap_all(map: HashMap<String, String>) -> HashMap<String, SecretString> {
    map.into_iter().map(|(k, v)| (k, SecretString::new(v.into()))).collect()
}