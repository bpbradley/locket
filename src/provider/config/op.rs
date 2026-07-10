use crate::path::AbsolutePath;
use crate::provider::{
    AuthToken, ProviderError, Signature,
    references::{HasReference, OpReference},
};
use async_trait::async_trait;
use clap::Args;
use locket_derive::LayeredConfig;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct OpConfig {
    pub op_token: AuthToken,
    pub op_bridge: Option<AbsolutePath>,
}

impl HasReference for OpConfig {
    type Reference = OpReference;
}

#[async_trait]
impl Signature for OpConfig {
    async fn signature(&self) -> Result<u64, ProviderError> {
        self.op_token.signature().await
    }
}

#[derive(
    Args, Debug, Clone, Default, LayeredConfig, Deserialize, Serialize, PartialEq, Eq, Hash,
)]
#[serde(rename_all = "kebab-case")]
#[locket(try_into = "OpConfig")]
pub struct OpArgs {
    /// 1Password Service Account Token
    ///
    /// Either provide the token directly or via a file with `file:` prefix
    #[arg(long, env = "OP_SERVICE_ACCOUNT_TOKEN", hide_env_values = true)]
    pub op_token: Option<AuthToken>,

    /// Optional: Path to the locket-op-bridge binary
    ///
    /// Overrides automatic discovery, which prefers a bridge embedded
    /// in this binary and otherwise expects `locket-op-bridge` next to
    /// the locket executable. PATH is never searched.
    #[arg(long, env = "LOCKET_OP_BRIDGE")]
    #[locket(optional)]
    pub op_bridge: Option<AbsolutePath>,
}
