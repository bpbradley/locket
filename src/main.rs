use clap::Parser;
use locket::cmd::{Cli, Command, healthcheck, run};
use sysexits::ExitCode;

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.cmd {
        Command::Run(args) => run::run(args),
        Command::Healthcheck(args) => healthcheck::healthcheck(args),
    }
}
