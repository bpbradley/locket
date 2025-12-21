//! Secrets provider abstractions and implementations.
//!
//! This module defines the `SecretsProvider` trait,
//! which abstracts over different backend secret management services for batch
//! resolution of secret references.
//!
//! It also provides implementations for specific providers
//! and a selection mechanism to choose the provider at runtime
use async_trait::async_trait;
#[cfg(feature = "bws")]
use bws::{BwsConfig, BwsProvider};
use clap::{Args, ValueEnum};
#[cfg(feature = "connect")]
use connect::{OpConnectConfig, OpConnectProvider};
#[cfg(feature = "op")]
use op::{OpConfig, OpProvider};
use secrecy::{ExposeSecret, SecretString};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

#[cfg(not(any(feature = "op", feature = "connect", feature = "bws")))]
compile_error!("At least one provider feature must be enabled (e.g. --features op,connect)");

#[cfg(feature = "bws")]
mod bws;
#[cfg(feature = "connect")]
mod connect;
mod macros;
#[cfg(feature = "op")]
mod op;

// Re-export alias that is more expressive while internally remaining descriptive
pub use ProviderSelection as Provider;

#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    /// Network or API errors
    #[error("network request failed: {0}")]
    Network(#[source] Box<dyn std::error::Error + Send + Sync>),

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

/// Abstraction for a backend service that resolves secret references.
#[async_trait]
pub trait SecretsProvider: Send + Sync {
    /// Batch resolve a list of secret references.
    ///
    /// The input is a slice of unique keys found in the templates (e.g., `["op://vault/item/field", ...]`).
    /// The implementation should return a Map of `{ Reference -> SecretValue }`.
    ///
    /// If a reference cannot be resolved, it should simply be omitted from the result map
    /// rather than returning an error, allowing the template renderer to leave the tag unresolved.
    async fn fetch_map(
        &self,
        references: &[&str],
    ) -> Result<HashMap<String, SecretString>, ProviderError>;

    /// Returns `true` if the key string matches the syntax this provider supports.
    ///
    /// This is used to filter keys before attempting to fetch them.
    fn accepts_key(&self, key: &str) -> bool;
}

#[derive(Copy, Clone, Debug, ValueEnum)]
pub enum ProviderKind {
    /// 1Password Service Account
    #[cfg(feature = "op")]
    Op,
    /// 1Password Connect Provider
    #[cfg(feature = "connect")]
    OpConnect,
    /// Bitwarden Secrets Provider
    #[cfg(feature = "bws")]
    Bws,
}

#[derive(Args, Debug, Clone)]
pub struct ProviderSelection {
    /// Secrets provider backend to use.
    #[arg(long = "provider", env = "SECRETS_PROVIDER", value_enum)]
    pub kind: ProviderKind,

    /// Provider-specific configuration
    #[command(flatten, next_help_heading = "Provider Configuration")]
    pub cfg: ProviderConfig,
}

impl ProviderSelection {
    /// Build a runtime provider from configuration
    pub async fn build(&self) -> Result<Arc<dyn SecretsProvider>, ProviderError> {
        match self.kind {
            #[cfg(feature = "op")]
            ProviderKind::Op => Ok(Arc::new(OpProvider::new(self.cfg.op.clone()).await?)),
            #[cfg(feature = "connect")]
            ProviderKind::OpConnect => Ok(Arc::new(
                OpConnectProvider::new(self.cfg.connect.clone()).await?,
            )),
            #[cfg(feature = "bws")]
            ProviderKind::Bws => Ok(Arc::new(BwsProvider::new(self.cfg.bws.clone()).await?)),
        }
    }
}

#[derive(Args, Debug, Clone, Default)]
pub struct ProviderConfig {
    #[cfg(feature = "op")]
    #[command(flatten, next_help_heading = "1Password (op)")]
    pub op: OpConfig,
    #[cfg(feature = "connect")]
    #[command(flatten, next_help_heading = "1Password Connect")]
    pub connect: OpConnectConfig,
    #[cfg(feature = "bws")]
    #[command(flatten, next_help_heading = "Bitwarden Secrets Provider")]
    pub bws: BwsConfig,
}

/// A wrapper around `SecretString` which allows constructing from either a direct token or a file path.
///
/// It can be trivially constructed by passing a secret string, or it will attempt to resolve the token from the file if provided.
#[derive(Debug, Clone, Default)]
pub struct AuthToken {
    token: SecretString,
}

impl AuthToken {
    /// Simple wrapper for SecretString
    pub fn new(token: SecretString) -> Self {
        Self { token }
    }
    /// Attempt to create an AuthToken from either a direct token or a file path. If a token is directly passed, it takes precedence.
    /// Context must be provided for error messages.
    pub fn try_new(
        token: Option<SecretString>,
        file: Option<PathBuf>,
        context: &str,
    ) -> Result<Self, ProviderError> {
        match (&token, &file) {
            (Some(tok), _) => Ok(Self { token: tok.clone() }),
            (None, Some(path)) => {
                let content = std::fs::read_to_string(path).map_err(|e| {
                    ProviderError::InvalidConfig(format!(
                        "failed to read {} token file {:?}: {}",
                        context, path, e
                    ))
                })?;

                let trimmed = content.trim();
                if trimmed.is_empty() {
                    return Err(ProviderError::InvalidConfig(format!(
                        "{} token file {:?} is empty",
                        context, path
                    )));
                }

                Ok(Self {
                    token: SecretString::new(trimmed.to_owned().into()),
                })
            }
            _ => Err(ProviderError::InvalidConfig(format!(
                "Missing: {}",
                context
            ))),
        }
    }
}

/// Allows exposing the inner secret string using ExposeSecret from `secrecy` crate
impl ExposeSecret<str> for AuthToken {
    fn expose_secret(&self) -> &str {
        self.token.expose_secret()
    }
}
