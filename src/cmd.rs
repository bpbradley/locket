use crate::{
    health::StatusFile,
    logging::Logger,
    provider::{Provider, SecretsProvider},
    secrets::{Secrets, manager::{SecretsOpts, SecretSources}},
};
use clap::{Args, Parser, Subcommand, ValueEnum};

#[derive(Parser, Debug)]
#[command(name = "secret-sidecar")]
#[command(version, about = "Materialize secrets from environment or templates", long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub cmd: Command,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Run Secret Sidecar
    Run(RunArgs),

    /// Healthcheck
    Healthcheck(HealthArgs),
}

#[derive(Default, Copy, Clone, Debug, ValueEnum)]
pub enum RunMode {
    /// Run once and exit
    OneShot,
    /// Watch for changes and re-apply
    #[default]
    Watch,
    /// Run once and then park to keep the process alive
    Park,
}

#[derive(Args, Debug)]
pub struct RunArgs {
    /// Run mode
    #[arg(long = "mode", env = "RUN_MODE", value_enum, default_value_t = RunMode::Watch)]
    pub mode: RunMode,

    /// Status file path
    #[command(flatten)]
    pub status_file: StatusFile,

    /// Secret Management Configuration
    #[command(flatten)]
    pub secrets: SecretsOpts,

    /// Secret Sources
    #[command(flatten)]
    pub values: SecretSources,

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
        self.secrets.validate()?;
        Ok(Secrets::new(self.secrets.clone()).with_values(self.values.load()))
    }
}

pub mod healthcheck;
pub mod run;
