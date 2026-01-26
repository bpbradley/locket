use crate::provider::{
    AuthToken, ConcurrencyLimit, ProviderError, Signature,
    bws::BwsUrl,
    references::{BwsReference, HasReference},
};
use async_trait::async_trait;
use clap::Args;
use locket_derive::LayeredConfig;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct BwsConfig {
    pub bws_api_url: BwsUrl,
    pub bws_identity_url: BwsUrl,
    pub bws_max_concurrent: ConcurrencyLimit,
    pub bws_user_agent: String,
    pub bws_token: AuthToken,
}

impl HasReference for BwsConfig {
    type Reference = BwsReference;
}

#[async_trait]
impl Signature for BwsConfig {
    async fn signature(&self) -> Result<u64, ProviderError> {
        self.bws_token.signature().await
    }
}

#[derive(
    Args, Debug, Clone, LayeredConfig, Deserialize, Serialize, Default, PartialEq, Eq, Hash,
)]
#[serde(rename_all = "kebab-case")]
#[locket(try_into = "BwsConfig")]
pub struct BwsArgs {
    /// Bitwarden API URL
    #[arg(long, env = "BWS_API_URL")]
    #[locket(default = "https://api.bitwarden.com")]
    pub bws_api_url: Option<BwsUrl>,

    /// Bitwarden Identity URL
    #[arg(long, env = "BWS_IDENTITY_URL")]
    #[locket(default = "https://identity.bitwarden.com")]
    pub bws_identity_url: Option<BwsUrl>,

    /// Maximum number of concurrent requests to Bitwarden Secrets Manager
    #[arg(long, env = "BWS_MAX_CONCURRENT")]
    #[locket(default = ConcurrencyLimit::new(20))]
    pub bws_max_concurrent: Option<ConcurrencyLimit>,

    /// BWS User Agent
    #[arg(long, env = "BWS_USER_AGENT")]
    #[locket(default = env!("CARGO_PKG_NAME"))]
    pub bws_user_agent: Option<String>,

    /// Bitwarden Machine Token
    ///
    /// Either provide the token directly or via a file with `file:` prefix
    #[arg(long, env = "BWS_MACHINE_TOKEN", hide_env_values = true)]
    pub bws_token: Option<AuthToken>,
}
