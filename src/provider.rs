//! Secrets provider implementation
//!
//! Providers will inject secrets from templates
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
    /// Secrets provider
    #[arg(long = "provider", env = "SECRETS_PROVIDER", value_enum)]
    pub kind: ProviderKind,

    /// Provider-specific configuration
    #[command(flatten, next_help_heading = "Provider Configuration")]
    pub cfg: ProviderConfig,
}

impl ProviderSelection {
    /// Build a runtime provider from configuration
    pub async fn build(&self) -> Result<Box<dyn SecretsProvider>, ProviderError> {
        match self.kind {
            #[cfg(feature = "op")]
            ProviderKind::Op => Ok(Box::new(OpProvider::new(self.cfg.op.clone())?)),
            #[cfg(feature = "connect")]
            ProviderKind::OpConnect => {
                Ok(Box::new(OpConnectProvider::new(self.cfg.connect.clone())?))
            }
            #[cfg(feature = "bws")]
            ProviderKind::Bws => Ok(Box::new(BwsProvider::new(self.cfg.bws.clone()).await?)),
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
