use crate::{
    error::LocketError,
    health::{HealthError, StatusFile},
};
use clap::Args;

#[derive(Args, Debug)]
pub struct HealthArgs {
    /// Status file path
    #[command(flatten)]
    pub status_file: StatusFile,
}

pub fn healthcheck(args: HealthArgs) -> Result<(), LocketError> {
    if args.status_file.is_ready() {
        Ok(())
    } else {
        Err(LocketError::Health(HealthError::Unhealthy))
    }
}
