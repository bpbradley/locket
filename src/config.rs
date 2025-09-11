use crate::logging::{LogFormat, LogLevel};
use clap::Args;
use std::path::PathBuf;

#[derive(Default, Args, Debug, Clone)]
pub struct Config {
    /// Templates directory
    #[arg(long, env = "TEMPLATES_DIR", default_value = "/templates")]
    pub templates_dir: PathBuf,

    /// Output directory
    #[arg(long, env = "OUTPUT_DIR", default_value = "/run/secrets")]
    pub output_dir: PathBuf,

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

    /// Allow inject fallback to raw copy
    #[arg(long, env = "INJECT_FALLBACK_COPY", default_value_t = true)]
    pub inject_fallback_copy: bool,

    /// Log format
    #[arg(long, env = "LOG_FORMAT", value_enum, default_value_t = LogFormat::Text)]
    pub log_format: LogFormat,

    /// Log level
    #[arg(long, env = "LOG_LEVEL", value_enum, default_value_t = LogLevel::Info)]
    pub log_level: LogLevel,
}
