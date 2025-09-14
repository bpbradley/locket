use clap::Parser;
use secret_sidecar::cmd;

fn main() -> anyhow::Result<()> {
    let cli = cmd::Cli::parse();
    match cli.cmd {
        cmd::Command::Run(args) => cmd::run::run(args),
        cmd::Command::Healthcheck(args) => {
            let code = cmd::healthcheck::healthcheck(args)?;
            std::process::exit(code);
        }
    }
}
