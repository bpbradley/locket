use crate::cmd::config::healthcheck::HealthArgs;
use crate::{error::LocketError, health::HealthError};

pub fn healthcheck(args: HealthArgs) -> Result<(), LocketError> {
    if args.status_file.is_ready() {
        Ok(())
    } else {
        Err(LocketError::Health(HealthError::Unhealthy))
    }
}
