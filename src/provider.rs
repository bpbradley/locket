//! Secrets provider abstractions and implementations.
//!
//! This module defines the `SecretsProvider` trait,
//! which abstracts over different backend secret management services for batch
//! resolution of secret references.
//!
//! It also provides implementations for specific providers
//! and a selection mechanism to choose the provider at runtime
use crate::path::CanonicalPath;
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
use std::num::NonZeroUsize;
use std::sync::Arc;

#[cfg(not(any(feature = "op", feature = "connect", feature = "bws")))]
compile_error!("At least one provider feature must be enabled (e.g. --features op,connect,bws)");

#[cfg(feature = "bws")]
mod bws;
#[cfg(feature = "connect")]
mod connect;
mod macros;
#[cfg(feature = "op")]
mod op;
mod references;

pub use references::{ReferenceParseError, ReferenceParser, SecretReference};

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

    /// Invalid ID format
    #[error("invalid id: {0}")]
    InvalidId(String),

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

    /// URL parse error
    #[error("url error: {0}")]
    Url(#[from] url::ParseError),
}

/// Abstraction for a backend service that resolves secret references.
#[async_trait]
pub trait SecretsProvider: ReferenceParser + Send + Sync {
    /// Batch resolve a list of secret references.
    ///
    /// The input is a slice of unique keys found in the templates (e.g., `["op://vault/item/field", ...]`).
    /// The implementation should return a Map of `{ Reference -> SecretValue }`.
    ///
    /// If a reference cannot be resolved, it should simply be omitted from the result map
    /// rather than returning an error, allowing the template renderer to leave the tag unresolved.
    async fn fetch_map(
        &self,
        references: &[SecretReference],
    ) -> Result<HashMap<SecretReference, SecretString>, ProviderError>;
}

/// Provider backend configuration
pub enum Provider {
    #[cfg(feature = "op")]
    Op(OpConfig),
    #[cfg(feature = "connect")]
    Connect(OpConnectConfig),
    #[cfg(feature = "bws")]
    Bws(BwsConfig),
}

impl Provider {
    pub async fn build(self) -> Result<Arc<dyn SecretsProvider>, ProviderError> {
        let provider: Arc<dyn SecretsProvider> = match self {
            #[cfg(feature = "op")]
            Self::Op(c) => Arc::new(OpProvider::new(c).await?),
            #[cfg(feature = "connect")]
            Self::Connect(c) => Arc::new(OpConnectProvider::new(c).await?),
            #[cfg(feature = "bws")]
            Self::Bws(c) => Arc::new(BwsProvider::new(c).await?),
        };

        Ok(provider)
    }
}

impl From<ProviderArgs> for Provider {
    fn from(args: ProviderArgs) -> Self {
        match args.kind {
            #[cfg(feature = "op")]
            ProviderKind::Op => Self::Op(args.config.op),

            #[cfg(feature = "connect")]
            ProviderKind::OpConnect => Self::Connect(args.config.connect),

            #[cfg(feature = "bws")]
            ProviderKind::Bws => Self::Bws(args.config.bws),
        }
    }
}

#[derive(Args, Debug, Clone)]
pub struct ProviderArgs {
    /// Secrets provider backend to use.
    #[arg(long = "provider", env = "SECRETS_PROVIDER", value_enum)]
    kind: ProviderKind,

    /// Provider-specific configuration
    #[command(flatten, next_help_heading = "Provider Configuration")]
    config: ProviderConfigs,
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

#[derive(Args, Debug, Clone, Default)]
pub struct ProviderConfigs {
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

pub enum TokenSource {
    Literal(SecretString),
    File(CanonicalPath),
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
    pub fn try_from_source(source: TokenSource, context: &str) -> Result<Self, ProviderError> {
        match source {
            TokenSource::Literal(secret) => {
                if secret.expose_secret().trim().is_empty() {
                    return Err(ProviderError::InvalidConfig(format!(
                        "{} token literal is empty",
                        context
                    )));
                }
                Ok(Self { token: secret })
            }
            TokenSource::File(canon_path) => {
                let content = std::fs::read_to_string(canon_path.as_path()).map_err(|e| {
                    ProviderError::InvalidConfig(format!(
                        "failed to read {} token file {:?}: {}",
                        context, canon_path, e
                    ))
                })?;

                let trimmed = content.trim();
                if trimmed.is_empty() {
                    return Err(ProviderError::InvalidConfig(format!(
                        "{} token file {:?} is empty",
                        context, canon_path
                    )));
                }

                Ok(Self {
                    token: SecretString::new(trimmed.to_owned().into()),
                })
            }
        }
    }
}

/// Allows exposing the inner secret string using ExposeSecret from `secrecy` crate
impl ExposeSecret<str> for AuthToken {
    fn expose_secret(&self) -> &str {
        self.token.expose_secret()
    }
}

#[derive(Debug, Clone, Copy)]
struct ConcurrencyLimit(NonZeroUsize);

impl ConcurrencyLimit {
    pub const fn new(limit: usize) -> Self {
        if limit == 0 {
            // Static string panic is supported in const fn
            panic!("ConcurrencyLimit: value must be greater than 0");
        }
        Self(NonZeroUsize::new(limit).unwrap())
    }
    pub fn into_inner(self) -> usize {
        self.0.get()
    }
}

impl Default for ConcurrencyLimit {
    fn default() -> Self {
        Self::new(20)
    }
}

impl std::str::FromStr for ConcurrencyLimit {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let val: usize = s.parse().map_err(|_| "not a number")?;
        NonZeroUsize::new(val)
            .map(Self)
            .ok_or_else(|| "Concurrency must be > 0".to_string())
    }
}

impl std::fmt::Display for ConcurrencyLimit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}
