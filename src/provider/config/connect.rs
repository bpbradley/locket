use crate::provider::{
    AuthToken, ConcurrencyLimit, ProviderError, ServerUrl, Signature,
    references::{HasReference, OpReference},
};
use async_trait::async_trait;
use clap::Args;
use locket_derive::LayeredConfig;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ConnectConfig {
    pub connect_host: ServerUrl,
    pub connect_token: AuthToken,
    pub connect_max_concurrent: ConcurrencyLimit,
}

impl HasReference for ConnectConfig {
    type Reference = OpReference;
}

#[async_trait]
impl Signature for ConnectConfig {
    async fn signature(&self) -> Result<u64, ProviderError> {
        self.connect_token.signature().await
    }
}

#[derive(
    Args, Debug, Clone, LayeredConfig, Deserialize, Serialize, Default, PartialEq, Eq, Hash,
)]
#[locket(try_into = "ConnectConfig")]
#[serde(rename_all = "kebab-case")]
pub struct ConnectArgs {
    /// 1Password Connect Host HTTP(S) URL
    #[arg(long, env = "OP_CONNECT_HOST")]
    pub connect_host: Option<ServerUrl>,

    /// 1Password Connect Token
    ///
    /// Either provide the token directly or via a file with `file:` prefix
    #[arg(long, env = "OP_CONNECT_TOKEN", hide_env_values = true)]
    pub connect_token: Option<AuthToken>,

    /// Maximum allowed concurrent requests to Connect API
    #[arg(long, env = "OP_CONNECT_MAX_CONCURRENT")]
    #[locket(default = ConcurrencyLimit::new(20))]
    pub connect_max_concurrent: Option<ConcurrencyLimit>,
}
