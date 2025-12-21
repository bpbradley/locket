//! Main entry point for the secret-sidecar binary.
//!
//! This binary provides the `locket` command-line interface,
//! and otherwise serves as a thin dispatch layer for `locket`
use clap::Parser;
use locket::cmd;
use locket::cmd::{Cli, Command};
use locket::error::LocketError;
use std::process::{ExitCode, Termination};

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    let result: Result<(), LocketError> = match cli.cmd {
        Command::Run(args) => cmd::run(*args).await,
        #[cfg(feature = "exec")]
        Command::Exec(args) => cmd::exec(*args).await,
        Command::Healthcheck(args) => cmd::healthcheck(args),
        #[cfg(feature = "compose")]
        Command::Compose(args) => cmd::compose(*args).await,
        #[cfg(feature = "compose")]
        Command::DockerCliPluginMetadata => {
            let metadata = serde_json::json!({
                "SchemaVersion": "0.1.0",
                "Vendor": "Brian Bradley",
                "Version": env!("CARGO_PKG_VERSION"),
                "ShortDescription": "Secret management for Docker Compose",
                "URL": "https://github.com/bpbradley/locket"
            });
            println!("{}", metadata);
            Ok(())
        }
    };
    match result {
        Ok(_) => ExitCode::SUCCESS,
        Err(e) => e.report(),
    }
}
