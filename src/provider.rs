//! Secrets provider implementation
//!
//! Providers will inject secrets from templates
use crate::provider::op::{OpConfig, OpProvider};
use async_trait::async_trait;
use clap::{Args, ValueEnum};
use std::collections::HashMap;

#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    /// Network or API errors
    #[error("network request failed: {0}")]
    Network(#[source] anyhow::Error),

    /// The secret reference was valid syntax, but the provider couldn't find it
    #[error("secret not found: {0}")]
    NotFound(String),

    /// Authentication/Authorization failures
    #[error("access denied: {0}")]
    Unauthorized(String),

    /// Rate limiting
    #[error("rate limited")]
    RateLimit,

    /// Generic error
    #[error("{0}")]
    Other(String),

    /// Invalid or missing configuration
    #[error("invalid config: {0}")]
    InvalidConfig(String),

    /// Fs/Io errors
    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    /// External command errors
    #[error("command '{program}' failed with status {status:?}: {stderr}")]
    Exec {
        program: &'static str,
        status: Option<i32>,
        stderr: String,
    },
}

#[async_trait]
pub trait SecretsProvider: Send + Sync {
    /// Batch resolve a list of secret references.
    ///
    /// The input is a list of keys found in the template (e.g. "op://vault/item/field").
    /// Returns a Map of { Reference -> SecretValue }.
    async fn fetch_map(
        &self,
        references: &[&str],
    ) -> Result<HashMap<String, String>, ProviderError>;

    /// Returns true if the key string looks like a reference this provider supports.
    fn accepts_key(&self, key: &str) -> bool;
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
