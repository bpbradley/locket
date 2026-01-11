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
use clap::{Args, ValueEnum};
use locket_derive::LayeredConfig;
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize, Serializer};
use std::num::NonZeroUsize;
use std::sync::Arc;
use std::{collections::HashMap, str::FromStr};

#[cfg(not(any(feature = "op", feature = "connect", feature = "bws")))]
compile_error!("At least one provider feature must be enabled (e.g. --features op,connect,bws)");

#[cfg(feature = "bws")]
mod bws;
pub mod config;
#[cfg(feature = "connect")]
mod connect;
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
#[derive(Debug, Clone)]
pub enum Provider {
    #[cfg(feature = "op")]
    Op(config::op::OpConfig),
    #[cfg(feature = "connect")]
    Connect(config::connect::ConnectConfig),
    #[cfg(feature = "bws")]
    Bws(config::bws::BwsConfig),
}

impl Provider {
    pub async fn build(self) -> Result<Arc<dyn SecretsProvider>, ProviderError> {
        let provider: Arc<dyn SecretsProvider> = match self {
            #[cfg(feature = "op")]
            Self::Op(c) => Arc::new(op::OpProvider::new(c).await?),
            #[cfg(feature = "connect")]
            Self::Connect(c) => Arc::new(connect::OpConnectProvider::new(c).await?),
            #[cfg(feature = "bws")]
            Self::Bws(c) => Arc::new(bws::BwsProvider::new(c).await?),
        };

        Ok(provider)
    }
}

#[derive(Args, Debug, Clone, LayeredConfig, Deserialize, Serialize, Default)]
#[serde(rename_all = "kebab-case")]
pub struct ProviderArgs {
    /// Secrets provider backend to use.
    #[arg(long, env = "SECRETS_PROVIDER")]
    pub provider: Option<ProviderKind>,

    /// Provider-specific configuration
    #[command(flatten)]
    #[serde(flatten)]
    pub config: ProviderConfigs,
}

impl TryFrom<ProviderArgs> for Provider {
    type Error = crate::error::LocketError;

    fn try_from(args: ProviderArgs) -> Result<Self, Self::Error> {
        let kind = args.provider.ok_or_else(|| {
            crate::config::ConfigError::Validation(
                "Missing required argument: --provider <kind>".into(),
            )
        })?;

        match kind {
            #[cfg(feature = "bws")]
            ProviderKind::Bws => Ok(Provider::Bws(args.config.bws.try_into()?)),
            #[cfg(feature = "op")]
            ProviderKind::Op => Ok(Provider::Op(args.config.op.try_into()?)),
            #[cfg(feature = "connect")]
            ProviderKind::OpConnect => Ok(Provider::Connect(args.config.connect.try_into()?)),
        }
    }
}

#[derive(Copy, Clone, Debug, ValueEnum, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
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

#[derive(Args, Debug, Clone, LayeredConfig, Deserialize, Serialize, Default)]
#[serde(rename_all = "kebab-case")]
pub struct ProviderConfigs {
    #[cfg(feature = "op")]
    #[command(flatten, next_help_heading = "1Password (op)")]
    #[serde(flatten)]
    pub op: config::op::OpArgs,

    #[cfg(feature = "connect")]
    #[command(flatten, next_help_heading = "1Password Connect")]
    #[serde(flatten)]
    pub connect: config::connect::ConnectArgs,

    #[cfg(feature = "bws")]
    #[command(flatten, next_help_heading = "Bitwarden Secrets Provider")]
    #[serde(flatten)]
    pub bws: config::bws::BwsArgs,
}

/// A wrapper around `SecretString` which allows constructing from either a direct token or a file path.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
#[serde(try_from = "String")]
pub struct AuthToken(SecretString);

impl AuthToken {
    /// Simple wrapper for SecretString
    pub fn new(token: SecretString) -> Self {
        Self(token)
    }

    pub fn try_from_file(path: CanonicalPath) -> Result<Self, ProviderError> {
        let content = std::fs::read_to_string(path.as_path()).map_err(|e| {
            ProviderError::InvalidConfig(format!("failed to read token file {:?}: {}", path, e))
        })?;

        let trimmed = content.trim();
        if trimmed.is_empty() {
            return Err(ProviderError::InvalidConfig(format!(
                "token file {:?} is empty",
                path
            )));
        }

        Ok(Self(SecretString::new(trimmed.to_owned().into())))
    }
}

impl Serialize for AuthToken {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        // Do not expose the actual token.
        serializer.serialize_str("[REDACTED]")
    }
}

impl AsRef<SecretString> for AuthToken {
    fn as_ref(&self) -> &SecretString {
        &self.0
    }
}

impl TryFrom<String> for AuthToken {
    type Error = ProviderError;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        s.parse()
    }
}

impl FromStr for AuthToken {
    type Err = ProviderError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim();
        if s.is_empty() {
            return Err(ProviderError::InvalidConfig(
                "auth token is empty".to_string(),
            ));
        }
        // If token path starts with "file:", treat it as a file path
        if let Some(path) = s.strip_prefix("file:") {
            let cleaned = path.strip_prefix("//").unwrap_or(path);
            let canon = CanonicalPath::try_new(cleaned).map_err(|e| {
                ProviderError::InvalidConfig(format!(
                    "failed to resolve token file '{:?}': {}",
                    path, e
                ))
            })?;
            Self::try_from_file(canon)
        } else {
            Ok(Self(SecretString::new(s.to_owned().into())))
        }
    }
}

/// Allows exposing the inner secret string using ExposeSecret from `secrecy` crate
impl ExposeSecret<str> for AuthToken {
    fn expose_secret(&self) -> &str {
        self.0.expose_secret()
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub struct ConcurrencyLimit(NonZeroUsize);

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
