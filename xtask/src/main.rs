use clap::{Parser, Subcommand};
use xtask::docs::DocGenerator;
use xtask::plugin::PluginConfigArgs;

#[derive(Parser)]
struct Xtask {
    #[command(subcommand)]
    cmd: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Generate configuration tables in markdown format from clap definitions
    Docs(DocGenerator),
    /// Generate plugin configuration from for volume driver from clap definitions
    Plugin(PluginConfigArgs),
}

fn main() -> anyhow::Result<()> {
    let args = Xtask::parse();

    match args.cmd {
        Commands::Docs(docs) => docs.generate(),
        Commands::Plugin(config) => config.generate(),
    }
}
