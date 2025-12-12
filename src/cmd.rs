use clap::{Parser, Subcommand};

#[cfg(feature = "compose")]
mod compose;
mod healthcheck;
mod run;
#[cfg(feature = "exec")]
mod exec;

#[cfg(feature = "compose")]
pub use compose::compose;
pub use healthcheck::healthcheck;
pub use run::run;
#[cfg(feature = "exec")]
pub use exec::exec;

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
    Run(Box<run::RunArgs>),

    #[cfg(feature = "exec")]
    Exec(exec::ExecArgs),

    /// Checks the health of the sidecar agent, determined by the state of materialized secrets.
    /// Exits with code 0 if all known secrets are materialized, otherwise exits with non-zero exit code.
    Healthcheck(healthcheck::HealthArgs),
    /// Docker Compose provider API
    #[cfg(feature = "compose")]
    Compose(Box<compose::ComposeArgs>),

    /// Docker CLI plugin metadata command
    #[cfg(feature = "compose")]
    #[command(name = "docker-cli-plugin-metadata", hide = true)]
    DockerCliPluginMetadata,
}
