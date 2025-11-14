use crate::{
    health::StatusFile,
    logging::Logger,
    provider::{Provider, SecretsProvider},
    secrets::{Secrets, SecretsOpts},
};
use clap::{Args, Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(name = "secret-sidecar")]
#[command(version, about = "Materialize secrets from environment or templates", long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub cmd: Command,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Run the sidecar
    Run(RunArgs),

    /// Healthcheck: exit 0 if secrets ready, else exit 1
    Healthcheck(HealthArgs),
}

#[derive(Args, Debug)]
pub struct RunArgs {
    /// Run a single sync and exit
    #[arg(long)]
    pub once: bool,

    /// Watch for changes
    #[arg(long, env = "WATCH", default_value_t = true)]
    pub watch: bool,

    /// Status file path
    #[command(flatten)]
    pub status_file: StatusFile,

    /// Secret Management Configuration
    #[command(flatten)]
    pub secrets: SecretsOpts,

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

impl RunArgs {
    pub fn provider(&self) -> anyhow::Result<Box<dyn SecretsProvider>> {
        Ok(self.provider.build()?)
    }
    pub fn secrets(&self) -> anyhow::Result<Secrets> {
        Ok(self.secrets.build()?)
    }
}

pub mod healthcheck;
pub mod run;
