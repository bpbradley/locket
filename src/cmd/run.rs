// run.rs
use super::{RunArgs, RunMode};
use crate::{health::StatusFile, watch::FsWatcher};
use sysexits::ExitCode;
use tracing::{debug, error};

pub fn run(args: RunArgs) -> ExitCode {
    if let Err(e) = args.logger.init() {
        error!(error=%e, "init logging failed");
        return ExitCode::CantCreat;
    }
    debug!(?args, "effective config");

    let status: &StatusFile = &args.status_file;
    status.clear().unwrap_or_else(|e| {
        error!(error=%e, "failed to clear status file on startup");
    });

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

    match secrets.collisions() {
        Ok(()) => {}
        Err(e) => {
            error!(error=%e, "secret destination collisions detected");
            return ExitCode::Config;
        }
    };

    if let Err(e) = secrets.inject_all(provider.as_ref()) {
        error!(error=%e, "inject_all failed");
        return ExitCode::IoErr;
    }

    debug!("injection complete; creating status file");
    if let Err(e) = status.mark_ready() {
        error!(error=%e, "failed to write status file");
        return ExitCode::IoErr;
    }

    match args.mode {
        RunMode::OneShot => ExitCode::Ok,
        RunMode::Park => {
            tracing::info!("parking... (ctrl-c to exit)");
            let (tx, rx) = std::sync::mpsc::channel();

            ctrlc::set_handler(move || {
                let _ = tx.send(());
            })
            .expect("Error setting Ctrl-C handler");

            let _ = rx.recv();

            tracing::info!("shutdown signal received. exiting.");
            ExitCode::Ok
        }
        RunMode::Watch => {
            let mut watcher = FsWatcher::new(&mut secrets, provider.as_ref());
            match watcher.run() {
                Ok(()) => ExitCode::Ok,
                Err(e) => {
                    error!(error=%e, "watch errored");
                    ExitCode::IoErr
                }
            }
        }
    }
}
