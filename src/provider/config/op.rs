use crate::path::AbsolutePath;
use crate::provider::AuthToken;
use clap::Args;
use locket_derive::LayeredConfig;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default)]
pub struct OpConfig {
    pub op_token: AuthToken,
    pub op_config_dir: Option<AbsolutePath>,
}

#[derive(Args, Debug, Clone, Default, LayeredConfig, Deserialize, Serialize)]
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
