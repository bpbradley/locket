use crate::provider::{ProviderError, SecretsProvider};
use crate::write;
use clap::ValueEnum;
use std::fs;
use std::path::{Path, PathBuf};
use tempfile;
use thiserror::Error;
use tracing::{debug, info, warn};

#[derive(Debug, Error)]
pub enum SecretError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("provider error: {0}")]
    Provider(#[from] crate::provider::ProviderError),

    #[error("source path missing: {0:?}")]
    SourceMissing(PathBuf),

    #[error("destination {dst:?} is inside source {src:?}")]
    Loop { src: PathBuf, dst: PathBuf },

    #[error("source {src:?} is inside destination {dst:?}")]
    Destructive { src: PathBuf, dst: PathBuf },

    #[error("Path is forbidden in source: {0:?}")]
    Forbidden(PathBuf),

    #[error(
        "collision detected: '{dst:?}' is targeted by multiple sources: '{first:?}' and '{second:?}'"
    )]
    Collision {
        first: String,
        second: String,
        dst: PathBuf,
    },

    #[error(
        "structure conflict: {blocker:?} maps to '{blocker_path:?}', which blocks {blocked:?} from writing to '{blocked_path:?}'"
    )]
    StructureConflict {
        blocker: String,
        blocker_path: PathBuf,
        blocked: String,
        blocked_path: PathBuf,
    },

    // Fallback for generic injection errors
    #[error("injection failed: {source}")]
    InjectionFailed {
        #[source]
        source: crate::provider::ProviderError,
    },

    #[error("dst has no parent: {0}")]
    NoParent(PathBuf),
}

#[derive(Copy, Clone, Debug, ValueEnum, Default)]
pub enum InjectFailurePolicy {
    Error,
    #[default]
    CopyUnmodified,
    Ignore,
}

/// Something that can be injected to a destination path.
pub trait Injectable {
    /// Label used for logging
    fn label(&self) -> &str;
    /// Destination path on disk.
    fn dst(&self) -> &Path;
    /// copy implementation for fallback on injection error
    fn copy(&self) -> Result<(), SecretError>;
    /// secret injection with provider
    fn injector(&self, provider: &dyn SecretsProvider, dst: &Path) -> Result<(), ProviderError>;
    /// Generic secret injection with failure policy
    fn inject(
        &self,
        policy: InjectFailurePolicy,
        provider: &dyn SecretsProvider,
    ) -> Result<(), SecretError> {
        info!(src=?self.label(), dst=?self.dst(), "injecting secret");

        let parent = self
            .dst()
            .parent()
            .ok_or_else(|| SecretError::NoParent(self.dst().to_path_buf()))?;

        fs::create_dir_all(parent)?;

        let tmp_out = tempfile::Builder::new()
            .prefix(".tmp.")
            .tempfile_in(parent)?
            .into_temp_path();

        match self.injector(provider, tmp_out.as_ref()) {
            Ok(()) => {
                write::atomic_move(tmp_out.as_ref(), self.dst())?;
                Ok(())
            }
            Err(e) => match policy {
                InjectFailurePolicy::Error => Err(SecretError::InjectionFailed { source: e }),
                InjectFailurePolicy::CopyUnmodified => {
                    warn!(
                        src=?self.label(),
                        dst=?self.dst(),
                        error=?e,
                        "injection failed; falling back to raw copy for secret"
                    );
                    self.copy()?;
                    Ok(())
                }
                InjectFailurePolicy::Ignore => {
                    warn!(
                        src=?self.label(),
                        dst=?self.dst(),
                        error=?e,
                        "injection failed; ignoring"
                    );
                    Ok(())
                }
            },
        }
    }
    /// Remove the secret from disk.
    fn remove(&self) -> Result<(), SecretError> {
        let dst = self.dst();
        debug!(dst=?dst, exists=?dst.exists(), "removing secret");
        if dst.exists() {
            fs::remove_file(dst)?;
        }
        Ok(())
    }
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
    fn label(&self) -> &str {
        self.src.to_str().unwrap_or("<invalid utf8>")
    }
    fn dst(&self) -> &Path {
        &self.dst
    }
    fn copy(&self) -> Result<(), SecretError> {
        write::atomic_copy(&self.src, &self.dst)?;
        Ok(())
    }
    fn injector(&self, provider: &dyn SecretsProvider, dst: &Path) -> Result<(), ProviderError> {
        provider.inject(&self.src, dst)?;
        Ok(())
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
    fn label(&self) -> &str {
        &self.label
    }
    fn dst(&self) -> &Path {
        &self.dst
    }
    fn copy(&self) -> Result<(), SecretError> {
        write::atomic_write(&self.dst, self.template.as_bytes())?;
        Ok(())
    }
    fn injector(&self, provider: &dyn SecretsProvider, dst: &Path) -> Result<(), ProviderError> {
        provider.inject_from_bytes(self.template.as_bytes(), dst)?;
        Ok(())
    }
}
