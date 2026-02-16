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
    #[cfg(target_os = "linux")]
    /// Generate plugin configuration from for volume driver from clap definitions
    Plugin(xtask::plugin::PluginConfigArgs),
}

fn main() -> anyhow::Result<()> {
    let args = Xtask::parse();

    match args.cmd {
        Commands::Docs(docs) => docs.generate(),
        #[cfg(target_os = "linux")]
        Commands::Plugin(config) => config.generate(),
    }
}
