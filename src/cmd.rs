//! CLI entry point and subcommand dispatch.
//!
//! This module defines the top-level `locket` command-line interface.
//! It dispatches execution to specific handlers:
//!
//! * **Inject**: Sidecar mode (`locket inject`).
//! * **Exec**: Process injection wrapper (`locket exec`).
//! * **Healthcheck**: Health probe for sidecar
//! * **Compose**: Docker Compose provider integration.

use clap::{Parser, Subcommand};
#[cfg(feature = "compose")]
mod compose;
#[cfg(feature = "exec")]
mod exec;
mod healthcheck;
mod inject;

#[cfg(feature = "compose")]
pub use compose::compose;
#[cfg(feature = "exec")]
pub use exec::exec;
pub use healthcheck::healthcheck;
pub use inject::inject;

#[derive(Parser, Debug)]
#[command(name = "locket")]
#[command(version, about = "Materialize secrets from environment or templates", long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub cmd: Command,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Inject secrets from secret references into files and directories.
    ///
    /// Example:
    ///
    /// ```sh
    /// locket inject --provider bws --bws-token=file:/path/to/token \ # Select the BWS provider
    ///     --out /run/secrets/locket \ # Default output directory
    ///     --secret=/path/to/secrets.yaml \ # An anonymous secret file, placed in `/run/secrets/locket/secrets.yaml`
    ///     --secret=auth_key=@key.pem \ # A named secret file, placed in `/run/secrets/locket/auth_key`
    ///     --map ./tpl:/run/secrets/locket/mapped \ # Maps all templates in `./tpl/` directory to secrets in `/run/secrets/locket/mapped`
    /// ```
    #[clap(verbatim_doc_comment)]
    Inject(Box<inject::InjectArgs>),

    /// Execute a command with secrets injected into the process environment.
    /// and optionally materialize secrets from template files.
    ///
    /// Example:
    ///
    /// ```sh
    /// locket exec --provider bws --bws-token=file:/path/to/token \
    ///     -e locket.env -e OVERRIDE={{ reference }}
    ///     --map ./tpl/config:/app/config \
    ///     -- docker compose up -d
    /// ```
    #[cfg(feature = "exec")]
    #[clap(verbatim_doc_comment)]
    Exec(Box<exec::ExecArgs>),

    /// Checks the health of the sidecar agent, determined by the state of materialized secrets.
    ///
    /// Exits with code 0 if all known secrets are materialized, otherwise exits with non-zero exit code.
    #[clap(verbatim_doc_comment)]
    Healthcheck(healthcheck::HealthArgs),
    /// Docker Compose provider API
    #[cfg(feature = "compose")]
    Compose(Box<compose::ComposeArgs>),

    /// Docker CLI plugin metadata command
    #[cfg(feature = "compose")]
    #[command(name = "docker-cli-plugin-metadata", hide = true)]
    DockerCliPluginMetadata,
}
