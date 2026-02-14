//! Secrets provider abstractions and implementations.
//!
//! This module defines the `SecretsProvider` trait,
//! which abstracts over different backend secret management services for batch
//! resolution of secret references.
//!
//! It also provides implementations for specific providers
//! and a selection mechanism to choose the provider at runtime

use async_trait::async_trait;
use clap::{Args, ValueEnum};
use locket_derive::LayeredConfig;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

#[cfg(not(any(
    feature = "op",
    feature = "connect",
    feature = "bws",
    feature = "infisical"
)))]
compile_error!(
    "At least one provider feature must be enabled (e.g. --features op,connect,bws,infisical)"
);

#[cfg(feature = "bws")]
mod bws;
pub mod config;
#[cfg(feature = "connect")]
mod connect;
#[cfg(feature = "infisical")]
mod infisical;
pub mod managed;
#[cfg(feature = "op")]
mod op;
mod references;
mod types;

use managed::{ManagedProvider, ProviderFactory};
pub use references::{ReferenceParseError, ReferenceParser, SecretReference};
pub use types::{AuthToken, ConcurrencyLimit, TokenSource};

/// Trait for configuration structs that can produce a "signature" representing their content's freshness.
#[async_trait]
pub trait Signature: Send + Sync {
    /// Returns a hash of the configuration's sensitive content.
    ///
    /// For static configuration, this may assume a constant or default.
    /// For file-based configuration, this should check the file content.
    async fn signature(&self) -> Result<u64, ProviderError>;
}

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
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Provider {
    #[cfg(feature = "op")]
    Op(config::op::OpConfig),
    #[cfg(feature = "connect")]
    Connect(config::connect::ConnectConfig),
    #[cfg(feature = "bws")]
    Bws(config::bws::BwsConfig),
    #[cfg(feature = "infisical")]
    Infisical(config::infisical::InfisicalConfig),
}

impl Provider {
    pub async fn build(self) -> Result<Arc<dyn SecretsProvider>, ProviderError> {
        let managed = ManagedProvider::new(self).await?;
        Ok(Arc::new(managed))
    }
}

#[async_trait]
impl Signature for Provider {
    async fn signature(&self) -> Result<u64, ProviderError> {
        match self {
            #[cfg(feature = "op")]
            Self::Op(c) => c.signature().await,
            #[cfg(feature = "connect")]
            Self::Connect(c) => c.signature().await,
            #[cfg(feature = "bws")]
            Self::Bws(c) => c.signature().await,
            #[cfg(feature = "infisical")]
            Self::Infisical(c) => c.signature().await,
        }
    }
}

impl ReferenceParser for Provider {
    fn parse(&self, raw: &str) -> Option<SecretReference> {
        match self {
            #[cfg(feature = "op")]
            Self::Op(cfg) => cfg.parse(raw),
            #[cfg(feature = "connect")]
            Self::Connect(cfg) => cfg.parse(raw),
            #[cfg(feature = "bws")]
            Self::Bws(cfg) => cfg.parse(raw),
            #[cfg(feature = "infisical")]
            Self::Infisical(cfg) => cfg.parse(raw),
        }
    }
}

#[async_trait]
impl ProviderFactory for Provider {
    async fn create(&self) -> Result<Arc<dyn SecretsProvider>, ProviderError> {
        let provider: Arc<dyn SecretsProvider> = match self {
            #[cfg(feature = "op")]
            Self::Op(c) => Arc::new(op::OpProvider::new(c.clone()).await?),
            #[cfg(feature = "connect")]
            Self::Connect(c) => Arc::new(connect::OpConnectProvider::new(c.clone()).await?),
            #[cfg(feature = "bws")]
            Self::Bws(c) => Arc::new(bws::BwsProvider::new(c.clone()).await?),
            #[cfg(feature = "infisical")]
            Self::Infisical(c) => Arc::new(infisical::InfisicalProvider::new(c.clone()).await?),
        };
        Ok(provider)
    }
}

#[derive(
    Args, Debug, Clone, Hash, PartialEq, Eq, LayeredConfig, Deserialize, Serialize, Default,
)]
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
        use crate::config::ApplyDefaults;
        let args = args.apply_defaults();

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
            #[cfg(feature = "infisical")]
            ProviderKind::Infisical => Ok(Provider::Infisical(args.config.infisical.try_into()?)),
        }
    }
}

#[derive(Copy, Clone, Debug, Hash, PartialEq, Eq, ValueEnum, Deserialize, Serialize)]
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
    /// Infisical Secrets Provider
    #[cfg(feature = "infisical")]
    Infisical,
}

#[derive(
    Args, Debug, Clone, Hash, PartialEq, Eq, LayeredConfig, Deserialize, Serialize, Default,
)]
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

    #[cfg(feature = "infisical")]
    #[command(flatten, next_help_heading = "Infisical Secrets Provider")]
    #[serde(flatten)]
    pub infisical: config::infisical::InfisicalArgs,
}
