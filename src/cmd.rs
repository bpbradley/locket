use crate::{
    health::StatusFile,
    logging::Logger,
    provider::{Provider, SecretsProvider},
    secrets::{SecretValues, Secrets, SecretsOpts},
    watch::WatcherOpts,
    write::FileWriter,
};
use clap::{Args, Parser, Subcommand, ValueEnum};

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
    Run(RunArgs),

    /// Checks the health of the sidecar agent, determined by the state of materialized secrets.
    /// Exits with code 0 if all known secrets are materialized, otherwise exits with non-zero exit code.
    Healthcheck(HealthArgs),
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
    #[command(flatten)]
    pub values: SecretValues,

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

impl RunArgs {
    pub fn provider(&self) -> anyhow::Result<Box<dyn SecretsProvider>> {
        Ok(self.provider.build()?)
    }
    pub fn secrets(&self) -> anyhow::Result<Secrets> {
        self.secrets.validate()?;
        Ok(Secrets::new(self.secrets.clone())
            .with_values(self.values.load())
            .with_writer(self.writer.clone()))
    }
}

#[derive(Args, Debug)]
pub struct HealthArgs {
    /// Status file path
    #[command(flatten)]
    pub status_file: StatusFile,
}

pub mod healthcheck;
pub mod run;
