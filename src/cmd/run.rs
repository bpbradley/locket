// run.rs
use super::{RunArgs, RunMode};
use crate::watch;
use sysexits::ExitCode;
use tracing::{debug, error};

pub fn run(args: RunArgs) -> ExitCode {
    if let Err(e) = args.logger.init() {
        error!(error=%e, "init logging failed");
        return ExitCode::CantCreat;
    }
    debug!(?args, "effective config");

    let provider = match args.provider() {
        Ok(p) => p,
        Err(e) => {
            error!(error=%e, "invalid provider configuration");
            return ExitCode::Config;
        }
    };

    let mut secrets = match args.secrets() {
        Ok(s) => s,
        Err(e) => {
            error!(error=%e, "failed collecting secrets from config");
            return ExitCode::Config;
        }
    };

    let conflicts = secrets.collisions();
    if !conflicts.is_empty() {
        error!(
            ?conflicts,
            "duplicate destination paths for secrets (files or values)"
        );
        return ExitCode::DataErr;
    }

    if let Err(e) = secrets.inject_all(provider.as_ref()) {
        error!(error=%e, "inject_all failed");
        return ExitCode::IoErr;
    }

    debug!("injection complete; creating status file");
    if let Err(e) = args.status_file.mark_ready() {
        error!(error=%e, "failed to write status file");
        return ExitCode::IoErr;
    }

    match args.mode {
        RunMode::OneShot => ExitCode::Ok,
        RunMode::Park => {
            std::thread::park();
            ExitCode::Ok
        }
        RunMode::Watch => match watch::run_watch(args, &mut secrets, provider.as_ref()) {
            Ok(()) => ExitCode::Ok,
            Err(e) => {
                error!(error=%e, "watch errored");
                ExitCode::IoErr
            }
        },
    }
}
