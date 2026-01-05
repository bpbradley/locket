use crate::{
    error::LocketError,
    health::{HealthError, StatusFile},
};
use clap::Args;

#[derive(Args, Debug)]
pub struct HealthArgs {
    /// Status file path used for healthchecks
    #[arg(
        long = "status-file",
        env = "LOCKET_STATUS_FILE",
        default_value = StatusFile::default().to_string()
    )]
    pub status_file: StatusFile,
}

pub fn healthcheck(args: HealthArgs) -> Result<(), LocketError> {
    if args.status_file.is_ready() {
        Ok(())
    } else {
        Err(LocketError::Health(HealthError::Unhealthy))
    }
}
