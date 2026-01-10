use crate::health::StatusFile;
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
