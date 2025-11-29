use clap::{Parser, Subcommand};
use xtask::docs::DocGenerator;

#[derive(Parser)]
struct Xtask {
    #[command(subcommand)]
    cmd: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Generate configuration tables in markdown format from clap definitions
    Docs(DocGenerator),
}

fn main() -> anyhow::Result<()> {
    let args = Xtask::parse();

    match args.cmd {
        Commands::Docs(docs) => docs.generate(),
    }
}
