// run.rs
use super::RunArgs;
use crate::{health, logging, watch};
use sysexits::ExitCode;
use tracing::{debug, error};

pub fn run(args: RunArgs) -> ExitCode {
    if let Err(e) = logging::init(args.config.log_format, args.config.log_level) {
        error!(error=%e, "init logging failed");
        return ExitCode::CantCreat;
    }
    debug!(?args.config, "effective config");

    let provider = match args.provider() {
        Ok(p) => p,
        Err(e) => {
            error!(error=%e, "invalid provider configuration");
            return ExitCode::Config;
        }
    };

    let mut set = match args.secrets() {
        Ok(s) => s,
        Err(e) => {
            error!(error=%e, "failed collecting secrets from config");
            return ExitCode::Config;
        }
    };

    let conflicts = set.collisions();
    if !conflicts.is_empty() {
        error!(
            ?conflicts,
            "duplicate destination paths for secrets (files or values)"
        );
        return ExitCode::DataErr;
    }

    if let Err(e) = set.inject_all(provider.as_ref()) {
        error!(error=%e, "inject_all failed");
        return ExitCode::IoErr;
    }

    debug!("injection complete; creating status file");
    if let Err(e) = health::mark_ready(&args.config.status_file) {
        error!(error=%e, "failed to write status file");
        return ExitCode::IoErr;
    }

    if args.once {
        ExitCode::Ok
    } else if args.config.watch {
        match watch::run_watch(&args.config, &mut set, provider.as_ref()) {
            Ok(()) => ExitCode::Ok,
            Err(e) => {
                error!(error=%e, "watch errored");
                ExitCode::IoErr
            }
        }
    } else {
        std::thread::park();
        ExitCode::Ok
    }
}
