use crate::provider::{
    AuthToken, ConcurrencyLimit, ProviderError, Signature,
    references::{BaoReference, HasReference},
};
use async_trait::async_trait;
use clap::Args;
use locket_derive::LayeredConfig;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct BaoConfig {
    pub bao_url: url::Url,
    pub bao_namespace: Option<String>,
    pub bao_auth_mount: String,
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
    pub bao_url: Option<url::Url>,

    /// OpenBao / Vault namespace (Enterprise/OpenBao Namespaces feature)
    #[arg(long, env = "BAO_NAMESPACE")]
    #[locket(optional)]
    pub bao_namespace: Option<String>,

    /// Auth mount path where the AppRole auth method is enabled
    #[arg(long, env = "BAO_AUTH_MOUNT")]
    #[locket(default = "approle")]
    pub bao_auth_mount: Option<String>,

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
