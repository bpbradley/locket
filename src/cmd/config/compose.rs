use crate::logging::LogLevel;
use crate::provider::ProviderArgs;
use crate::secrets::Secret;
use clap::{Args, Subcommand};
#[derive(Args, Debug)]
pub struct ComposeArgs {
    /// Compose Project Name
    #[arg(long = "project-name", env = "COMPOSE_PROJECT_NAME")]
    pub project_name: String,

    /// Docker Compose provider API command
    #[command(subcommand)]
    pub cmd: ComposeCommand,
}

#[derive(Args, Debug)]
pub struct UpArgs {
    /// Provider configuration
    #[command(flatten)]
    pub provider: ProviderArgs,

    /// Files containing environment variables which may contain secret references
    #[arg(
        long,
        env = "LOCKET_ENV_FILE",
        value_name = "/path/to/.env",
        alias = "env_file",
        value_delimiter = ',',
        hide_env_values = true,
        help_heading = None,
        value_parser = crate::path::parse_secret_path,
        action = clap::ArgAction::Append,
    )]
    pub env_file: Vec<Secret>,

    /// Environment variable overrides which may contain secret references
    #[arg(
        long,
        short = 'e',
        env = "LOCKET_ENV",
        value_name = "KEY=VAL, KEY=@FILE or /path/to/.env",
        value_delimiter = ',',
        hide_env_values = true,
        help_heading = None,
        action = clap::ArgAction::Append,
    )]
    pub env: Vec<Secret>,

    /// Inject variables with their exact names, without the provider service name prefix
    ///
    /// By default Docker Compose prefixes injected variables with the provider service name,
    /// e.g. `SECRET` becomes `LOCKET_SECRET` for a service named `locket`. 
    /// Enable this to inject variables as-is. 
    /// Requires Docker Compose v5.2.0 or later, and avoiding name collisions between provider services becomes your responsibility
    #[arg(
        long,
        env = "LOCKET_RAW",
        value_name = "true|false",
        default_value_t = false,
        action = clap::ArgAction::Set,
        num_args = 0..=1,
        default_missing_value = "true",
        help_heading = None,
    )]
    pub raw: bool,

    /// Log level
    #[arg(long, env = "LOCKET_LOG_LEVEL", value_enum, default_value_t = LogLevel::Debug)]
    pub log_level: LogLevel,

    /// Service name from Docker Compose
    #[arg(help_heading = None)]
    pub service: String,
}

#[derive(Subcommand, Debug)]
pub enum ComposeCommand {
    /// Injects secrets into a Docker Compose service environment with `docker compose up`
    Up(Box<UpArgs>),
    /// Handler for Docker Compose `down`, but no-op because secrets are not persisted
    Down(Box<DownArgs>),
    /// Handler for Docker Compose `metadata` command so that docker can query plugin capabilities
    Metadata,
}

#[derive(Args, Debug)]
pub struct DownArgs {
    /// Log level
    #[arg(long, env = "LOCKET_LOG_LEVEL", value_enum, default_value_t = LogLevel::Debug)]
    pub log_level: LogLevel,
    /// Service name from Docker Compose
    #[arg(help_heading = None)]
    pub service: String,
}
