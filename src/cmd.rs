use crate::{
    health::StatusFile,
    logging::Logger,
    provider::Provider,
    secrets::{SecretArg, SecretsOpts},
    watch::WatcherOpts,
    write::FileWriter,
};
use clap::{Args, Parser, Subcommand, ValueEnum};

pub mod compose;
pub mod healthcheck;
pub mod run;

#[derive(Parser, Debug)]
#[command(name = "locket")]
#[command(version, about = "Materialize secrets from environment or templates", long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub cmd: Command,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Start the secret sidecar agent.
    /// All secrets will be collected and materialized according to configuration.
    Run(Box<RunArgs>),

    /// Checks the health of the sidecar agent, determined by the state of materialized secrets.
    /// Exits with code 0 if all known secrets are materialized, otherwise exits with non-zero exit code.
    Healthcheck(HealthArgs),

    /// Docker Compose provider API
    Compose(ComposeArgs),
}

#[derive(Default, Copy, Clone, Debug, ValueEnum)]
pub enum RunMode {
    /// Collect and materialize all secrets once and then exit
    OneShot,
    /// Continuously watch for changes on configured templates and update secrets as needed
    #[default]
    Watch,
    /// Run once and then park to keep the process alive
    Park,
}

#[derive(Args, Debug)]
pub struct RunArgs {
    /// Mode of operation
    #[arg(long = "mode", env = "LOCKET_RUN_MODE", value_enum, default_value_t = RunMode::Watch)]
    pub mode: RunMode,

    /// Status file path used for healthchecks
    #[command(flatten)]
    pub status_file: StatusFile,

    /// Secret Management Configuration
    #[command(flatten)]
    pub secrets: SecretsOpts,

    /// Secret Sources
    #[arg(
        long = "secret",
        env = "LOCKET_SECRETS",
        value_name = "label={{template}}",
        value_delimiter = ',',
        hide_env_values = true
    )]
    pub values: Vec<SecretArg>,

    /// Filesystem watcher options
    #[command(flatten)]
    pub watcher: WatcherOpts,

    /// File writing permissions
    #[command(flatten)]
    pub writer: FileWriter,

    /// Logging configuration
    #[command(flatten)]
    pub logger: Logger,

    /// Secrets provider selection
    #[command(flatten, next_help_heading = "Provider Configuration")]
    provider: Provider,
}

#[derive(Args, Debug)]
pub struct HealthArgs {
    /// Status file path
    #[command(flatten)]
    pub status_file: StatusFile,
}

#[derive(Args, Debug)]
pub struct ComposeArgs {
    /// Compose Project Name
    #[arg(long = "project-name", env = "COMPOSE_PROJECT_NAME")]
    pub project_name: String,

    /// Docker Compose provider API command
    #[command(subcommand)]
    pub cmd: ComposeCommand,
}

#[derive(Subcommand, Debug)]
pub enum ComposeCommand {
    /// Handler for Docker Compose 'up' command
    Up(compose::up::UpArgs),
    /// Handler for Docker Compose 'down' command
    Down,
    /// Handler for Docker Compose 'metadata' command
    Metadata,
}
