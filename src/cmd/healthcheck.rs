use crate::health::StatusFile;
use clap::Args;

#[derive(Args, Debug)]
pub struct HealthArgs {
    /// Status file path
    #[command(flatten)]
    pub status_file: StatusFile,
}

pub fn healthcheck(args: HealthArgs) -> Result<(), crate::error::LocketError> {
    if args.status_file.is_ready() {
        Ok(())
    } else {
        Err(crate::error::LocketError::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "status file not found",
        )))
    }
}
