use clap::{Args, Subcommand};

pub mod down;
pub mod meta;
pub mod up;

#[derive(Args, Debug)]
pub struct ComposeArgs {
    /// Compose Project Name
    #[arg(long = "project-name", env = "COMPOSE_PROJECT_NAME")]
    pub project_name: String,

    /// Docker Compose provider API command
    #[command(subcommand)]
    pub cmd: ComposeCommand,
}

#[derive(Subcommand, Debug)]
pub enum ComposeCommand {
    /// Injects secrets into a Docker Compose service environment with `docker compose up`
    Up(Box<up::UpArgs>),
    /// Handler for Docker Compose `down`, but no-op because secrets are not persisted
    Down,
    /// Handler for Docker Compose `metadata` command so that docker can query plugin capabilities
    Metadata,
}

pub async fn compose(args: ComposeArgs) -> sysexits::ExitCode {
    let project = args.project_name;
    match args.cmd {
        ComposeCommand::Up(args) => up::up(project, *args).await,
        ComposeCommand::Down => down::down(project).await,
        ComposeCommand::Metadata => meta::metadata(project).await,
    }
}
