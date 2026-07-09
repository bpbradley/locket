//! Shared types for managing Secret Providers

use super::ProviderError;
use crate::path::CanonicalPath;
use secrecy::SecretString;
use serde::{Deserialize, Serialize, Serializer};
use std::fmt;
use std::num::NonZeroUsize;
use std::str::FromStr;
use thiserror::Error;
use url::Url;

#[derive(Debug, Error)]
pub enum ServerUrlError {
    #[error("invalid url: {0}")]
    Parse(#[from] url::ParseError),

    #[error("invalid server url '{0}': scheme must be http or https")]
    Scheme(Url),

    #[error("invalid server url '{0}': query and fragment are not allowed")]
    Components(Url),
}

/// A provider API base URL: `http(s)://host[:port][/path-prefix]`.
///
/// The optional path prefix supports servers behind path-routing reverse
/// proxies.
///
/// Guaranteed hierarchical with no query or fragment, so endpoint URLs can
/// always be built from it. A trailing slash is normalized away at
/// construction so equal configurations compare and hash equal.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "Url", into = "Url")]
pub struct ServerUrl(Url);

impl ServerUrl {
    /// Builds an endpoint URL by appending path segments to the base URL,
    /// percent-encoding each segment as needed.
    pub fn endpoint<'a>(&self, segments: impl IntoIterator<Item = &'a str>) -> Url {
        let mut url = self.0.clone();
        url.path_segments_mut()
            .expect("http(s) urls are hierarchical")
            .pop_if_empty()
            .extend(segments);
        url
    }
}

impl TryFrom<Url> for ServerUrl {
    type Error = ServerUrlError;

    fn try_from(mut url: Url) -> Result<Self, Self::Error> {
        if !matches!(url.scheme(), "http" | "https") {
            return Err(ServerUrlError::Scheme(url));
        }
        if url.query().is_some() || url.fragment().is_some() {
            return Err(ServerUrlError::Components(url));
        }
        url.path_segments_mut()
            .expect("http(s) urls are hierarchical")
            .pop_if_empty();
        Ok(Self(url))
    }
}

impl FromStr for ServerUrl {
    type Err = ServerUrlError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Url::parse(s)?.try_into()
    }
}

impl From<ServerUrl> for Url {
    fn from(url: ServerUrl) -> Self {
        url.0
    }
}

impl fmt::Display for ServerUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_server_url_accepts_bare_host() {
        assert!(ServerUrl::from_str("https://bao.example.com:8200").is_ok());
        assert!(ServerUrl::from_str("http://127.0.0.1:8200/").is_ok());
    }

    #[test]
    fn test_server_url_rejects_non_http_schemes() {
        assert!(matches!(
            ServerUrl::from_str("data:text/plain,hello"),
            Err(ServerUrlError::Scheme(_))
        ));
        assert!(matches!(
            ServerUrl::from_str("unix:/var/run/bao.sock"),
            Err(ServerUrlError::Scheme(_))
        ));
        assert!(matches!(
            ServerUrl::from_str("not a url"),
            Err(ServerUrlError::Parse(_))
        ));
    }

    #[test]
    fn test_server_url_rejects_query_and_fragment() {
        for bad in [
            "https://example.com?query=1",
            "https://example.com/#fragment",
            "https://example.com/vault?query=1",
        ] {
            assert!(
                matches!(ServerUrl::from_str(bad), Err(ServerUrlError::Components(_))),
                "'{bad}' should be rejected"
            );
        }
    }

    #[test]
    fn test_server_url_endpoint_from_bare_host() {
        let url = ServerUrl::from_str("https://bao.example.com:8200").unwrap();
        let endpoint = url.endpoint(["v1", "secret", "data", "app"]);
        assert_eq!(
            endpoint.as_str(),
            "https://bao.example.com:8200/v1/secret/data/app"
        );
    }

    #[test]
    fn test_server_url_endpoint_appends_to_path_prefix() {
        let url = ServerUrl::from_str("https://example.com/vault").unwrap();
        let endpoint = url.endpoint(["v1", "secret", "data", "app"]);
        assert_eq!(
            endpoint.as_str(),
            "https://example.com/vault/v1/secret/data/app"
        );
    }

    #[test]
    fn test_server_url_normalizes_trailing_slash() {
        let with = ServerUrl::from_str("https://example.com/vault/").unwrap();
        let without = ServerUrl::from_str("https://example.com/vault").unwrap();
        assert_eq!(with, without);
        assert_eq!(
            with.endpoint(["v1"]).as_str(),
            without.endpoint(["v1"]).as_str()
        );
    }

    #[test]
    fn test_server_url_endpoint_encodes_segments() {
        let url = ServerUrl::from_str("https://example.com").unwrap();
        let endpoint = url.endpoint(["v1", "auth", "app role", "login"]);
        assert_eq!(
            endpoint.as_str(),
            "https://example.com/v1/auth/app%20role/login"
        );
    }
}
