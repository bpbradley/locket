use crate::provider::{
    AuthToken, ConcurrencyLimit, ProviderError, Signature,
    references::{BaoMount, BaoReference, HasReference},
};
use async_trait::async_trait;
use clap::Args;
use locket_derive::LayeredConfig;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;
use thiserror::Error;
use url::Url;

#[derive(Debug, Error)]
pub enum BaoConfigError {
    #[error("invalid url: {0}")]
    Url(#[from] url::ParseError),

    #[error("invalid bao url '{0}': scheme must be http or https")]
    Scheme(Url),

    #[error("invalid bao url '{0}': must be scheme://host[:port] with no path, query, or fragment")]
    Components(Url),

    #[error(
        "invalid namespace '{0}': expected non-empty '/' separated segments of visible ascii, relative to root (no leading '/')"
    )]
    Namespace(String),
}

/// The OpenBao / Vault server base URL: `http(s)://host[:port]`.
///
/// Guaranteed hierarchical with nothing but scheme and authority, so
/// endpoint URLs can always be built from it and nothing is silently
/// discarded.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "Url", into = "Url")]
pub struct BaoServerUrl(Url);

impl BaoServerUrl {
    /// Builds an endpoint URL from path segments.
    pub fn endpoint<'a>(&self, segments: impl IntoIterator<Item = &'a str>) -> Url {
        let mut url = self.0.clone();
        url.path_segments_mut()
            .expect("http(s) urls are hierarchical")
            .clear()
            .extend(segments);
        url
    }
}

impl TryFrom<Url> for BaoServerUrl {
    type Error = BaoConfigError;

    fn try_from(url: Url) -> Result<Self, Self::Error> {
        if !matches!(url.scheme(), "http" | "https") {
            return Err(BaoConfigError::Scheme(url));
        }
        if url.path() != "/" || url.query().is_some() || url.fragment().is_some() {
            return Err(BaoConfigError::Components(url));
        }
        Ok(Self(url))
    }
}

impl FromStr for BaoServerUrl {
    type Err = BaoConfigError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Url::parse(s)?.try_into()
    }
}

impl From<BaoServerUrl> for Url {
    fn from(url: BaoServerUrl) -> Self {
        url.0
    }
}

impl fmt::Display for BaoServerUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// An OpenBao / Vault namespace path (e.g. `admin/team1`).
///
/// Namespaces are hierarchical, '/' separated, and relative to root.
/// A trailing slash is normalized away so equal namespaces compare equal.
/// Segments are restricted to visible ascii, which also guarantees the
/// value is valid as an `X-Vault-Namespace` header.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct BaoNamespace(String);

impl BaoNamespace {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for BaoNamespace {
    type Error = BaoConfigError;

    fn try_from(mut value: String) -> Result<Self, Self::Error> {
        if value.ends_with('/') {
            value.pop();
        }
        let valid = !value.is_empty()
            && value
                .split('/')
                .all(|seg| !seg.is_empty() && seg.chars().all(|c| c.is_ascii_graphic()));
        if !valid {
            return Err(BaoConfigError::Namespace(value));
        }
        Ok(Self(value))
    }
}

impl FromStr for BaoNamespace {
    type Err = BaoConfigError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::try_from(s.to_string())
    }
}

impl From<BaoNamespace> for String {
    fn from(ns: BaoNamespace) -> Self {
        ns.0
    }
}

impl AsRef<str> for BaoNamespace {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for BaoNamespace {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct BaoConfig {
    pub bao_url: BaoServerUrl,
    pub bao_namespace: Option<BaoNamespace>,
    pub bao_auth_mount: BaoMount,
    pub bao_role_id: String,
    pub bao_secret_id: AuthToken,
    pub bao_max_concurrent: ConcurrencyLimit,
}

impl HasReference for BaoConfig {
    type Reference = BaoReference;
}

#[async_trait]
impl Signature for BaoConfig {
    async fn signature(&self) -> Result<u64, ProviderError> {
        self.bao_secret_id.signature().await
    }
}

#[derive(
    Args, Debug, Clone, LayeredConfig, Deserialize, Serialize, Default, PartialEq, Eq, Hash,
)]
#[serde(rename_all = "kebab-case")]
#[locket(try_into = "BaoConfig")]
pub struct BaoArgs {
    /// OpenBao / Vault server URL
    #[arg(long, env = "BAO_URL")]
    pub bao_url: Option<BaoServerUrl>,

    /// OpenBao / Vault namespace (Enterprise/OpenBao Namespaces feature)
    #[arg(long, env = "BAO_NAMESPACE")]
    #[locket(optional)]
    pub bao_namespace: Option<BaoNamespace>,

    /// Auth mount path where the AppRole auth method is enabled
    #[arg(long, env = "BAO_AUTH_MOUNT")]
    #[locket(default = "approle")]
    pub bao_auth_mount: Option<BaoMount>,

    /// AppRole Role ID
    #[arg(long, env = "BAO_ROLE_ID")]
    pub bao_role_id: Option<String>,

    /// AppRole Secret ID
    ///
    /// Either provide the value directly or via a file with `file:` prefix
    #[arg(long, env = "BAO_SECRET_ID", hide_env_values = true)]
    pub bao_secret_id: Option<AuthToken>,

    /// Maximum allowed concurrent requests to the OpenBao/Vault API
    #[arg(long, env = "BAO_MAX_CONCURRENT")]
    #[locket(default = ConcurrencyLimit::new(20))]
    pub bao_max_concurrent: Option<ConcurrencyLimit>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_server_url_accepts_bare_host() {
        assert!(BaoServerUrl::from_str("https://bao.example.com:8200").is_ok());
        assert!(BaoServerUrl::from_str("http://127.0.0.1:8200/").is_ok());
    }

    #[test]
    fn test_server_url_rejects_non_http_schemes() {
        // cannot-be-a-base urls parse as Urls but cannot host endpoint paths
        assert!(matches!(
            BaoServerUrl::from_str("data:text/plain,hello"),
            Err(BaoConfigError::Scheme(_))
        ));
        assert!(matches!(
            BaoServerUrl::from_str("unix:/var/run/bao.sock"),
            Err(BaoConfigError::Scheme(_))
        ));
        assert!(matches!(
            BaoServerUrl::from_str("not a url"),
            Err(BaoConfigError::Url(_))
        ));
    }

    #[test]
    fn test_server_url_rejects_extra_components() {
        for bad in [
            "https://bao.example.com/some/prefix",
            "https://bao.example.com?query=1",
            "https://bao.example.com/#fragment",
        ] {
            assert!(
                matches!(
                    BaoServerUrl::from_str(bad),
                    Err(BaoConfigError::Components(_))
                ),
                "'{bad}' should be rejected"
            );
        }
    }

    #[test]
    fn test_server_url_endpoint() {
        let url = BaoServerUrl::from_str("https://bao.example.com:8200").unwrap();
        let endpoint = url.endpoint(["v1", "auth", "app role", "login"]);
        assert_eq!(
            endpoint.as_str(),
            "https://bao.example.com:8200/v1/auth/app%20role/login"
        );
    }

    #[test]
    fn test_namespace_normalizes_trailing_slash() {
        let ns = BaoNamespace::from_str("admin/team1/").unwrap();
        assert_eq!(ns.as_str(), "admin/team1");
        assert_eq!(ns, BaoNamespace::from_str("admin/team1").unwrap());
    }

    #[test]
    fn test_namespace_rejects_malformed() {
        for bad in ["", "/admin", "admin//team1", "admin /team1", "admin\nteam1"] {
            assert!(
                BaoNamespace::from_str(bad).is_err(),
                "'{bad}' should be rejected"
            );
        }
    }
}
