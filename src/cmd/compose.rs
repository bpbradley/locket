pub mod down;
pub mod meta;
pub mod up;

pub async fn compose(args: super::ComposeArgs) -> sysexits::ExitCode {
    let project = args.project_name;
    match args.cmd {
        super::ComposeCommand::Up(args) => up::up(project, args).await,
        super::ComposeCommand::Down => down::down(project).await,
        super::ComposeCommand::Metadata => meta::metadata(project).await,
    }
}
