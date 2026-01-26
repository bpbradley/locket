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
use secrecy::SecretString;
use serde::{Deserialize, Serialize, Serializer};
use std::num::NonZeroUsize;
use std::sync::Arc;
use std::{collections::HashMap, str::FromStr};

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

use managed::{ManagedProvider, ProviderBuilder};
pub use references::{ReferenceParseError, ReferenceParser, SecretReference};

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
impl ProviderBuilder for Provider {
    async fn connect(&self) -> Result<Arc<dyn SecretsProvider>, ProviderError> {
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

/// A source for an authentication token.
#[derive(Debug, Clone)]
pub enum TokenSource {
    Literal(SecretString),
    File(CanonicalPath),
}

impl Eq for TokenSource {}

impl PartialEq for TokenSource {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Literal(l), Self::Literal(r)) => {
                use secrecy::ExposeSecret;
                l.expose_secret() == r.expose_secret()
            }
            (Self::File(l), Self::File(r)) => l == r,
            _ => false,
        }
    }
}

impl std::hash::Hash for TokenSource {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        use secrecy::ExposeSecret;
        std::mem::discriminant(self).hash(state);

        match self {
            TokenSource::Literal(s) => {
                // Must hash the secret content to use it as a cache identity.
                // This is safe because the hasher state is opaque and not persisted.
                // To avoid this completely, consider using a file-based token source
                // which will hash the file path instead.
                s.expose_secret().hash(state);
            }
            TokenSource::File(p) => {
                p.hash(state);
            }
        }
    }
}

/// A wrapper around `TokenSource` which allows constructing from either a direct token or a file path.
#[derive(Debug, Clone, Deserialize, Hash, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
#[serde(try_from = "String")]
pub struct AuthToken(TokenSource);

impl AuthToken {
    /// Create a new AuthToken from a SecretString
    pub fn new(token: SecretString) -> Self {
        Self(TokenSource::Literal(token))
    }

    /// Resolves the token source to the actual secret string.
    pub async fn resolve(&self) -> Result<SecretString, ProviderError> {
        match &self.0 {
            TokenSource::Literal(s) => Ok(s.clone()),
            TokenSource::File(path) => {
                let content = tokio::fs::read_to_string(path.as_path())
                    .await
                    .map_err(|e| {
                        ProviderError::InvalidConfig(format!(
                            "failed to read token file {:?}: {}",
                            path, e
                        ))
                    })?;
                let trimmed = content.trim();
                if trimmed.is_empty() {
                    return Err(ProviderError::InvalidConfig(format!(
                        "token file {:?} is empty",
                        path
                    )));
                }
                Ok(SecretString::new(trimmed.to_owned().into()))
            }
        }
    }

    /// Returns a signature of the current state of the token source.
    pub async fn signature(&self) -> Result<u64, ProviderError> {
        match &self.0 {
            TokenSource::Literal(_) => Ok(0),
            TokenSource::File(path) => {
                let content = tokio::fs::read_to_string(path.as_path())
                    .await
                    .map_err(|e| {
                        ProviderError::InvalidConfig(format!(
                            "failed to read token file for signature {:?}: {}",
                            path, e
                        ))
                    })?;
                use std::hash::{Hash, Hasher};
                let mut hasher = std::collections::hash_map::DefaultHasher::new();
                content.hash(&mut hasher);
                Ok(hasher.finish())
            }
        }
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
            Ok(Self(TokenSource::File(canon)))
        } else {
            Ok(Self(TokenSource::Literal(SecretString::new(
                s.to_owned().into(),
            ))))
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, Hash)]
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
