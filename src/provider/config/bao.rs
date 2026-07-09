use crate::provider::{
    AuthToken, ConcurrencyLimit, ProviderError, ServerUrl, Signature,
    references::{BaoMount, BaoReference, HasReference},
};
use async_trait::async_trait;
use clap::Args;
use locket_derive::LayeredConfig;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum BaoConfigError {
    #[error(
        "invalid namespace '{0}': expected non-empty '/' separated segments of visible ascii, relative to root (no leading '/')"
    )]
    Namespace(String),
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
    pub bao_url: ServerUrl,
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
    pub bao_url: Option<ServerUrl>,

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
