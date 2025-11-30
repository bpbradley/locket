use crate::provider::ProviderError;
use clap::ValueEnum;
use std::borrow::Cow;
use std::path::{Path, PathBuf};
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

    #[error("Relative paths are forbidden in source: {0:?}")]
    Forbidden(PathBuf),

    #[error("dst has no parent: {0}")]
    NoParent(PathBuf),
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
pub trait Injectable: Send + Sync {
    /// Destination path for injected secret
    fn dst(&self) -> &Path;

    /// label for logging and error messages
    fn label(&self) -> &str;

    /// Content as string
    fn content(&self) -> Result<Cow<'_, str>, SecretError>;
}

impl std::fmt::Display for dyn Injectable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Injectable(src='{}', dst='{}')",
            self.label(),
            self.dst().display()
        )
    }
}

/// Template file-backed secret
#[derive(Debug, Clone)]
pub struct SecretFile {
    /// Source path for template file
    src: PathBuf,
    /// Destination path for injected secret
    dst: PathBuf,
}

impl SecretFile {
    pub fn new(src: impl AsRef<Path>, dst: impl AsRef<Path>) -> Self {
        Self {
            src: src.as_ref().components().collect(),
            dst: dst.as_ref().components().collect(),
        }
    }
    pub fn src(&self) -> &Path {
        &self.src
    }
}

impl Injectable for SecretFile {
    fn dst(&self) -> &Path {
        &self.dst
    }
    fn label(&self) -> &str {
        self.src.to_str().unwrap_or("unknown")
    }
    fn content(&self) -> Result<Cow<'_, str>, SecretError> {
        let meta = std::fs::metadata(&self.src).map_err(SecretError::Io)?;

        // 10MB Limit. TODO: Make configurable
        if meta.len() > 10 * 1024 * 1024 {
            return Err(SecretError::SourceTooLarge {
                path: self.src.clone(),
                size: meta.len(),
                limit: 10 * 1024 * 1024,
            });
        }

        let content = std::fs::read_to_string(&self.src).map_err(SecretError::Io)?;
        Ok(Cow::Owned(content))
    }
}

/// Template string-backed secret
#[derive(Debug, Clone)]
pub struct SecretValue {
    dst: PathBuf,
    pub template: String,
    pub label: String,
}

impl SecretValue {
    pub fn new(dst: impl AsRef<Path>, template: impl AsRef<str>, label: impl AsRef<str>) -> Self {
        Self {
            dst: dst.as_ref().components().collect(),
            template: template.as_ref().to_string(),
            label: label.as_ref().to_string(),
        }
    }
}

impl Injectable for SecretValue {
    fn dst(&self) -> &Path {
        &self.dst
    }
    fn label(&self) -> &str {
        &self.label
    }
    fn content(&self) -> Result<Cow<'_, str>, SecretError> {
        Ok(Cow::Borrowed(&self.template))
    }
}
