use crate::path::AbsolutePath;
use crate::provider::AuthToken;
use async_trait::async_trait;
use clap::Args;
use locket_derive::LayeredConfig;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct OpConfig {
    pub op_token: AuthToken,
    pub op_config_dir: Option<AbsolutePath>,
}

#[async_trait]
impl crate::provider::Signature for OpConfig {
    async fn signature(&self) -> Result<u64, crate::provider::ProviderError> {
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

    /// Optional: Path to 1Password config directory
    ///
    /// Defaults to standard op config locations if not provided,
    /// e.g. `$XDG_CONFIG_HOME/op`
    #[arg(long, env = "OP_CONFIG_DIR")]
    #[locket(optional)]
    pub op_config_dir: Option<AbsolutePath>,
}
