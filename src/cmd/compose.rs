//! Handlers for Docker Compose Provider service
//!
//! Provider services are expected to handle `up` and `down` commands,
//! to be invoked by `docker compose` CLI prior to starting or stopping services.
//! This also implemented the optional `metadata` command, which allows Docker
//! to query the provider for its capabilities.
//! The metadata is derived from clap configuration on-demand.
pub mod down;
pub mod meta;
pub mod up;

use super::config::compose::{ComposeArgs, ComposeCommand};

pub async fn compose(args: ComposeArgs) -> Result<(), crate::error::LocketError> {
    let project = args.project_name;
    match args.cmd {
        ComposeCommand::Up(args) => up::up(project, *args).await,
        ComposeCommand::Down => down::down(project).await,
        ComposeCommand::Metadata => meta::metadata(project).await,
    }
}
