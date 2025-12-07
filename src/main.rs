use clap::Parser;
#[cfg(feature = "compose")]
use locket::cmd::compose;
use locket::cmd::{Cli, Command, healthcheck, run};
use sysexits::ExitCode;

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.cmd {
        Command::Run(args) => run::run(*args).await,
        Command::Healthcheck(args) => healthcheck::healthcheck(args),
        #[cfg(feature = "compose")]
        Command::Compose(args) => compose::compose(*args).await,
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
            sysexits::ExitCode::Ok
        }
    }
}
