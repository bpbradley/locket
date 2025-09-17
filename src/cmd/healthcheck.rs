use super::HealthArgs;
use sysexits::ExitCode;

pub fn healthcheck(args: HealthArgs) -> ExitCode {
    if crate::health::is_ready(&args.status_file) {
        ExitCode::Ok
    } else {
        ExitCode::Unavailable
    }
}
