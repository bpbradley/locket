use clap::{ArgAction, Parser};

#[derive(Parser, Debug)]
#[command(name = "secret-sidecar")]
#[command(version, about = "Materialize secrets from environment or templates", long_about = None)]
pub struct Cli {
    /// Run a single sync then block (no watch yet)
    #[arg(long, action=ArgAction::SetTrue)]
    pub once: bool,

    /// Healthcheck: exit 0 if secrets are ready
    #[arg(long, action=ArgAction::SetTrue)]
    pub healthcheck: bool,

    /// Log format: text|json
    #[arg(long, value_name="FORMAT", default_value_t=String::from("text"))]
    pub log_format: String,

    /// Log level: trace|debug|info|warn|error
    #[arg(long, value_name="LEVEL", default_value_t=String::from("info"))]
    pub log_level: String,

    /// Templates directory
    #[arg(long, value_name = "PATH")]
    pub templates_dir: Option<String>,

    /// Output directory
    #[arg(long, value_name = "PATH")]
    pub output_dir: Option<String>,

    /// Status file path
    #[arg(long, value_name = "PATH")]
    pub status_file: Option<String>,

    /// Watch for changes
    #[arg(long, value_name="BOOL", value_parser=clap::value_parser!(bool))]
    pub watch: Option<bool>,

    /// Allow inject fallback to raw copy
    #[arg(long, value_name="BOOL", value_parser=clap::value_parser!(bool))]
    pub inject_fallback_copy: Option<bool>,
}
