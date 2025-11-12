use crate::{
    config::Config,
    provider::{Provider, SecretsProvider},
    secrets:: Secrets,
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

    /// Override config
    #[command(flatten)]
    pub config: Config,

    /// Secrets provider selection
    #[command(flatten, next_help_heading = "Provider Configuration")]
    provider: Provider,
}

#[derive(Args, Debug)]
pub struct HealthArgs {
    /// Status file path
    #[arg(
        long,
        env = "STATUS_FILE",
        default_value = "/tmp/.secret-sidecar/ready"
    )]
    pub status_file: std::path::PathBuf,
}

impl RunArgs {
    pub fn provider(&self) -> anyhow::Result<Box<dyn SecretsProvider>> {
        Ok(self.provider.build()?)
    }
    pub fn secrets(&self) -> anyhow::Result<Secrets> {
        Ok(Secrets::build(self.config.secrets.clone())?)
    }
}

pub mod healthcheck;
pub mod run;
