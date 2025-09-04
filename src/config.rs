use clap::Args;
use serde::{Deserialize, Serialize};
#[derive(Args, Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Templates directory
    #[arg(long, env, default_value = "/templates")]
    pub templates_dir: String,

    /// Output directory
    #[arg(long, env, default_value = "/run/secrets")]
    pub output_dir: String,

    /// Status file path
    #[arg(long, env, default_value = "/tmp/.secret-sidecar/ready")]
    pub status_file: String,

    /// Watch for changes
    #[arg(long, env, default_value_t = true)]
    pub watch: bool,

    /// Allow inject fallback to raw copy
    #[arg(long, env, default_value_t = true)]
    pub inject_fallback_copy: bool,

    /// Log format: text|json
    #[arg(long, env, default_value = "text")]
    pub log_format: String,

    /// Log level: trace|debug|info|warn|error
    #[arg(long, env, default_value = "info")]
    pub log_level: String,

    /// Secrets provider
    #[arg(long, env, default_value = "op")]
    pub provider: String,
}
