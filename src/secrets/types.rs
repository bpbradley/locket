use crate::provider::ProviderError;
use crate::secrets::path::PathExt;
use clap::ValueEnum;
use std::borrow::Cow;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SecretError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("provider error: {0}")]
    Provider(#[from] ProviderError),

    #[error("input file too large: {size} bytes (limit: {limit} bytes): {path:?}")]
    SourceTooLarge {
        path: PathBuf,
        size: u64,
        limit: u64,
    },

    #[error("collision detected: '{dst:?}' is targeted by multiple sources")]
    Collision {
        first: String,
        second: String,
        dst: PathBuf,
    },

    #[error("structure conflict: {blocker:?} blocks {blocked:?}")]
    StructureConflict { blocker: String, blocked: String },

    #[error("source path missing: {0:?}")]
    SourceMissing(PathBuf),

    #[error("destination {dst:?} is inside source {src:?}")]
    Loop { src: PathBuf, dst: PathBuf },

    #[error("source {src:?} is inside destination {dst:?}")]
    Destructive { src: PathBuf, dst: PathBuf },

    #[error("dst has no parent: {0}")]
    NoParent(PathBuf),

    #[error("parse error: {0}")]
    Parse(String),
}

#[derive(Debug, Clone)]
pub struct Secret {
    pub key: String,
    pub source: SecretSource,
}

impl Secret {
    fn from_kv(key: String, val: String) -> Result<Self, SecretError> {
        // @file
        let source = if let Some(path) = val.strip_prefix('@') {
            SecretSource::file(path)?
        } else {
            SecretSource::literal(&key, val)
        };
        Ok(Self { key, source })
    }

    pub fn try_from_map(map: HashMap<String, String>) -> Result<Vec<Self>, SecretError> {
        map.into_iter().map(|(k, v)| Self::from_kv(k, v)).collect()
    }
}

impl FromStr for Secret {
    type Err = SecretError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (key, val) = s
            .split_once('=')
            .ok_or_else(|| SecretError::Parse(format!("expected KEY=VALUE, got '{}'", s)))?;

        // @ means load from file
        let source = if let Some(path) = val.strip_prefix('@') {
            SecretSource::File(PathBuf::from(path))
        } else {
            // Use key as the label for the literal
            SecretSource::literal(key, val)
        };

        Ok(Self {
            key: key.to_string(),
            source,
        })
    }
}

#[derive(Copy, Clone, Debug, ValueEnum, Default)]
pub enum InjectFailurePolicy {
    /// Injection failures are treated as errors and will abort the process
    Error,
    /// On injection failure, copy the unmodified secret to destination
    #[default]
    CopyUnmodified,
    /// On injection failure, just log a warning and proceed with the secret ignored
    Ignore,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SecretSource {
    /// Template loaded from a file path
    File(PathBuf),
    /// Template loaded from a string literal
    Literal {
        label: Option<String>,
        template: String,
    },
}

impl SecretSource {
    pub fn file(path: impl AsRef<Path>) -> Result<Self, SecretError> {
        let canon = path.as_ref().canon()?;
        Ok(Self::File(canon))
    }
    pub fn literal(label: impl Into<String>, text: impl Into<String>) -> Self {
        Self::Literal {
            label: Some(label.into()),
            template: text.into(),
        }
    }
    pub fn read(&self) -> SourceReader<'_> {
        SourceReader {
            source: self,
            max_size: u64::MAX,
        }
    }
    pub fn label(&self) -> Cow<'_, str> {
        match self {
            Self::File(p) => p.to_string_lossy(),
            Self::Literal { label, .. } => {
                Cow::Borrowed(label.as_deref().unwrap_or("inline-value"))
            }
        }
    }
}

impl From<PathBuf> for SecretSource {
    fn from(p: PathBuf) -> Self {
        Self::File(p)
    }
}

pub struct SourceReader<'a> {
    source: &'a SecretSource,
    max_size: u64,
}

impl<'a> SourceReader<'a> {
    pub fn limit(mut self, bytes: u64) -> Self {
        self.max_size = bytes;
        self
    }

    pub fn fetch(self) -> Result<Cow<'a, str>, SecretError> {
        match self.source {
            SecretSource::File(path) => {
                let meta = std::fs::metadata(path).map_err(SecretError::Io)?;
                if meta.len() > self.max_size {
                    return Err(SecretError::SourceTooLarge {
                        path: path.clone(),
                        size: meta.len(),
                        limit: self.max_size,
                    });
                }
                let c = std::fs::read_to_string(path).map_err(SecretError::Io)?;
                Ok(Cow::Owned(c))
            }
            SecretSource::Literal { template, .. } => Ok(Cow::Borrowed(template)),
        }
    }
}

#[derive(Debug, Clone)]
pub struct SecretFile {
    source: SecretSource,
    dest: PathBuf,
    max_size: u64,
}

impl SecretFile {
    pub fn from_file(
        src: impl AsRef<Path>,
        dest: impl AsRef<Path>,
        max_size: u64,
    ) -> Result<Self, SecretError> {
        Ok(Self {
            source: SecretSource::file(src)?,
            dest: dest.as_ref().absolute(),
            max_size,
        })
    }
    pub fn from_template(label: String, template: String, root: &Path) -> Self {
        let safe_name = sanitize_filename::sanitize(&label);
        let dest = root.absolute().join(safe_name);
        Self {
            source: SecretSource::literal(label, template),
            dest,
            max_size: u64::MAX,
        }
    }
    pub fn from_arg(arg: Secret, root: &Path, max_size: u64) -> Self {
        let safe_name = sanitize_filename::sanitize(&arg.key);
        Self {
            source: arg.source,
            dest: root.join(safe_name),
            max_size,
        }
    }
    pub fn dest(&self) -> &Path {
        &self.dest
    }
    pub fn source(&self) -> &SecretSource {
        &self.source
    }

    pub fn content(&self) -> Result<Cow<'_, str>, SecretError> {
        self.source.read().limit(self.max_size).fetch()
    }
}
