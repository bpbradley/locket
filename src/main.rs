use clap::Parser;
use locket::cmd::{Cli, Command, healthcheck, run};
use sysexits::ExitCode;

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.cmd {
        Command::Run(args) => run::run(*args).await,
        Command::Healthcheck(args) => healthcheck::healthcheck(args),
    }
}
