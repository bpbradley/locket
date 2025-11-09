use crate::logging::{LogFormat, LogLevel};
use crate::secrets::SecretsConfig;
use clap::Args;
use std::path::PathBuf;

#[derive(Default, Args, Debug, Clone)]
pub struct Config {
    /// Secret Management Configuration
    #[command(flatten)]
    pub secrets: SecretsConfig,

    /// Status file path
    #[arg(
        long,
        env = "STATUS_FILE",
        default_value = "/tmp/.secret-sidecar/ready"
    )]
    pub status_file: PathBuf,

    /// Watch for changes
    #[arg(long, env = "WATCH", default_value_t = true)]
    pub watch: bool,

    /// Log format
    #[arg(long, env = "LOG_FORMAT", value_enum, default_value_t = LogFormat::Text)]
    pub log_format: LogFormat,

    /// Log level
    #[arg(long, env = "LOG_LEVEL", value_enum, default_value_t = LogLevel::Info)]
    pub log_level: LogLevel,
}
