//! Secrets provider implementation
//!
//! Providers will inject secrets from templates
use crate::provider::op::{OpConfig, OpProvider};
use clap::{Args, ValueEnum};
use std::io::Write;
use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    /// File/FS/process spawning errors
    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    /// External command errors
    #[error("command '{program}' failed with status {status:?}: {stderr}")]
    Exec {
        program: &'static str,
        status: Option<i32>,
        stderr: String,
    },

    /// Invalid or missing configuration
    #[error("invalid config: {0}")]
    InvalidConfig(String),

    /// Generic error
    #[error("{0}")]
    Other(String),
}

pub trait SecretsProvider {
    fn inject(&self, src: &Path, dst: &Path) -> Result<(), ProviderError>;
    fn inject_from_bytes(&self, bytes: &[u8], dst: &Path) -> Result<(), ProviderError> {
        let parent = dst
            .parent()
            .ok_or_else(|| ProviderError::Other("destination directory doesn't exist".into()))?;
        let mut tmp = tempfile::Builder::new()
            .prefix(".src.")
            .tempfile_in(parent)?;
        tmp.write_all(bytes)?;
        tmp.as_file().sync_all()?;
        self.inject(tmp.path(), dst)
    }
}

#[derive(Copy, Clone, Debug, ValueEnum)]
pub enum ProviderKind {
    Op,
}

#[derive(Args, Debug, Clone)]
pub struct ProviderSelection {
    /// Secrets provider
    #[arg(long = "provider", env = "SECRETS_PROVIDER", value_enum)]
    pub kind: ProviderKind,

    /// Provider-specific configuration
    #[command(flatten, next_help_heading = "Provider Configuration")]
    pub cfg: ProviderConfig,
}

impl ProviderSelection {
    /// Build a runtime provider from configuration
    pub fn build(&self) -> Result<Box<dyn SecretsProvider>, ProviderError> {
        match self.kind {
            ProviderKind::Op => Ok(Box::new(OpProvider::new(self.cfg.op.clone())?)),
        }
    }
}

#[derive(Args, Debug, Clone, Default)]
pub struct ProviderConfig {
    #[command(flatten, next_help_heading = "1Password (op)")]
    pub op: OpConfig,
}

// Re-export alias that is more expressive while internally remaining descriptive
pub use ProviderSelection as Provider;

pub mod op;
