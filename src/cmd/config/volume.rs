use crate::logging::{Logger, LoggerArgs};
use crate::path::AbsolutePath;
use crate::volume::config::VolumeArgs;
use clap::Args;
use locket_derive::LayeredConfig;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub struct PluginConfig {
    pub socket: AbsolutePath,
    pub state_dir: AbsolutePath,
    pub runtime_dir: AbsolutePath,
    pub logger: Logger,
    pub volume_defaults: VolumeArgs,
}

#[derive(Args, Debug, Clone, Default, Serialize, Deserialize, LayeredConfig)]
#[serde(rename_all = "kebab-case")]
#[locket(try_into = "PluginConfig")]
pub struct PluginArgs {
    /// Path to the listening socket.
    #[arg(long, env = "LOCKET_PLUGIN_SOCKET")]
    #[locket(default = "/run/docker/plugins/locket.sock")]
    pub socket: Option<AbsolutePath>,

    /// Path to directory where state configuration is stored.
    ///
    /// This is where the plugin will store necessary data to reload configured volumes from cold start
    #[arg(long, env = "LOCKET_PLUGIN_STATE_DIR")]
    #[locket(default = "/var/lib/locket")]
    pub state_dir: Option<AbsolutePath>,

    /// Path to directory where runtime data is stored.
    ///
    /// This is where volumes are physically mounted on the host filesystem.
    #[arg(long, env = "LOCKET_PLUGIN_RUNTIME_DIR")]
    #[locket(default = "/var/lib/locket")]
    pub runtime_dir: Option<AbsolutePath>,

    #[command(flatten)]
    #[serde(flatten)]
    pub logger: LoggerArgs,

    #[command(flatten)]
    #[serde(flatten)]
    pub volume_defaults: VolumeArgs,
}
