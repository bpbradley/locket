use crate::provider::{ProviderError, SecretsProvider};
use crate::write;
use clap::ValueEnum;
use std::fs;
use std::path::{Path, PathBuf};
use tempfile;
use thiserror::Error;
use tracing::{info, warn};

#[derive(Debug, Error)]
pub enum SecretError {
    #[error("provider: {0}")]
    Provider(#[from] crate::provider::ProviderError),

    #[error("injection failed: {source}")]
    InjectionFailed {
        #[source]
        source: crate::provider::ProviderError,
    },

    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("dst has no parent: {0}")]
    NoParent(std::path::PathBuf),
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
        if dst.exists() {
            fs::remove_file(dst)?;
        }
        Ok(())
    }
}

/// Template file-backed secret
#[derive(Debug, Clone)]
pub struct SecretFile {
    /// Source path for template file
    pub src: PathBuf,
    /// Destination path for injected secret
    pub dst: PathBuf,
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
    pub dst: PathBuf,
    pub template: String,
    pub label: String,
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

/// Helper: sanitize label into a safe/consistent filename
pub fn sanitize_name(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for ch in raw.chars() {
        let lc = ch.to_ascii_lowercase();
        if lc.is_ascii_lowercase() || lc.is_ascii_digit() || matches!(lc, '.' | '_' | '-' | '/') {
            out.push(lc);
        } else {
            out.push('_');
        }
    }
    out
}

/// Construct a SecretValue from label + template.
pub fn value_source(output_root: &Path, label: &str, template: impl AsRef<str>) -> SecretValue {
    let sanitized = sanitize_name(label);
    let dst = output_root.join(&sanitized);
    SecretValue {
        dst,
        template: template.as_ref().to_string(),
        label: sanitized,
    }
}

pub fn collect_value_sources<L, T, I>(output_root: &Path, pairs: I) -> Vec<SecretValue>
where
    I: IntoIterator<Item = (L, T)>,
    L: AsRef<str>,
    T: AsRef<str>,
{
    pairs
        .into_iter()
        .map(|(label, template)| value_source(output_root, label.as_ref(), template))
        .collect()
}

pub fn collect_value_sources_from_env(output_root: &Path, prefix: &str) -> Vec<SecretValue> {
    let stripped = std::env::vars()
        .filter_map(|(k, v)| k.strip_prefix(prefix).map(|rest| (rest.to_string(), v)));
    collect_value_sources(output_root, stripped)
}
