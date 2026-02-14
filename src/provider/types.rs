//! Shared types for managing Secret Providers

use super::ProviderError;
use crate::path::CanonicalPath;
use secrecy::SecretString;
use serde::{Deserialize, Serialize, Serializer};
use std::num::NonZeroUsize;
use std::str::FromStr;

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
