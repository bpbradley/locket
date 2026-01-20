use crate::logging::{Logger, LoggerArgs};
use crate::path::AbsolutePath;
use crate::provider::{Provider, ProviderArgs};
use clap::Args;
use locket_derive::LayeredConfig;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub struct PluginConfig {
    pub socket: AbsolutePath,
    pub state_dir: AbsolutePath,
    pub runtime_dir: AbsolutePath,
    pub logger: Logger,
    pub provider: Provider,
}

#[derive(Args, Debug, Clone, Default, Serialize, Deserialize, LayeredConfig)]
#[serde(rename_all = "kebab-case")]
#[locket(try_into = "PluginConfig")]
pub struct PluginArgs {
    /// Path to the listening socket.
    #[arg(long, env = "LOCKET_PLUGIN_SOCKET")]
    #[locket(default = "/run/docker/plugins/locket.sock")]
    pub socket: Option<AbsolutePath>,

    #[arg(long, env = "LOCKET_PLUGIN_STATE_DIR")]
    #[locket(default = "/var/lib/locket")]
    pub state_dir: Option<AbsolutePath>,

    #[arg(long, env = "LOCKET_PLUGIN_RUNTIME_DIR")]
    #[locket(default = "/run/locket/volumes")]
    pub runtime_dir: Option<AbsolutePath>,

    #[command(flatten)]
    #[serde(flatten)]
    pub logger: LoggerArgs,

    #[command(flatten)]
    #[serde(flatten)]
    pub provider: ProviderArgs,
}
