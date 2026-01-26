//! Main entry point for the secret-sidecar binary.
//!
//! This binary provides the `locket` command-line interface,
//! and otherwise serves as a thin dispatch layer for `locket`
use clap::Parser;
use locket::cmd;
use locket::cmd::{Cli, Command};
use locket::error::LocketError;
use locket::logging::{LogFormat, LogLevel, Logger};
use std::process::{ExitCode, Termination};
mod exits;
use exits::LocketExitCode;

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(_) => ExitCode::SUCCESS,
        Err(e) => {
            // Fallback Logger
            // This should fail if a logger has already been initialized
            // allowing the already configured logger to handle the error reporting.
            let _ = Logger::new(LogFormat::Text, LogLevel::Info).init();
            LocketExitCode(e).report()
        }
    }
}

async fn run() -> Result<(), LocketError> {
    let cli = Cli::parse();
    match cli.cmd {
        Command::Inject(args) => {
            let config = args.load()?;
            cmd::inject(config).await
        }
        #[cfg(feature = "exec")]
        Command::Exec(args) => {
            let config = args.load()?;
            cmd::exec(config).await
        }
        Command::Healthcheck(args) => cmd::healthcheck(args),
        #[cfg(feature = "volume")]
        Command::Volume(args) => {
            let config = args.load()?;
            cmd::volume(config).await
        }
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
    }
}
