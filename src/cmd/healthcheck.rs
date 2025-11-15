use super::HealthArgs;
use sysexits::ExitCode;

pub fn healthcheck(args: HealthArgs) -> ExitCode {
    if args.status_file.is_ready() {
        ExitCode::Ok
    } else {
        ExitCode::Unavailable
    }
}
