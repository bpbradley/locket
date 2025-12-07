use crate::health::StatusFile;
use clap::Args;
use sysexits::ExitCode;

#[derive(Args, Debug)]
pub struct HealthArgs {
    /// Status file path
    #[command(flatten)]
    pub status_file: StatusFile,
}

pub fn healthcheck(args: HealthArgs) -> ExitCode {
    if args.status_file.is_ready() {
        ExitCode::Ok
    } else {
        ExitCode::Unavailable
    }
}
