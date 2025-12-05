// run.rs
use super::{RunArgs, RunMode};
use crate::{health::StatusFile, signal, watch::FsWatcher, secrets::Secrets};
use sysexits::ExitCode;
use tracing::{debug, error, info};

pub async fn run(args: RunArgs) -> ExitCode {
    if let Err(e) = args.logger.init() {
        error!(error=%e, "init logging failed");
        return ExitCode::CantCreat;
    }
    debug!(?args, "effective config");

    let RunArgs { 
        mut secrets,
        status_file, 
        values, 
        writer, 
        provider,
        watcher,
        mode,
        ..
    } = args;

    let status: &StatusFile = &status_file;
    status.clear().unwrap_or_else(|e| {
        error!(error=%e, "failed to clear status file on startup");
    });

    let provider = match provider.build().await {
        Ok(p) => p,
        Err(e) => {
            error!(error=%e, "invalid provider configuration");
            return ExitCode::Config;
        }
    };

    if let Err(e) = secrets.resolve() {
         error!(error=%e, "failed to resolve secret configuration");
         return ExitCode::Config;
    }

    let mut secrets = Secrets::new(secrets)
        .with_values(values.load())
        .with_writer(writer);

    match secrets.collisions() {
        Ok(()) => {}
        Err(e) => {
            error!(error=%e, "secret destination collisions detected");
            return ExitCode::Config;
        }
    };

    if let Err(e) = secrets.inject_all(provider.as_ref()).await {
        error!(error=%e, "inject_all failed");
        return ExitCode::IoErr;
    }

    debug!("injection complete; creating status file");
    if let Err(e) = status.mark_ready() {
        error!(error=%e, "failed to write status file");
        return ExitCode::IoErr;
    }

    match mode {
        RunMode::OneShot => ExitCode::Ok,
        RunMode::Park => {
            tracing::info!("parking... (ctrl-c to exit)");
            signal::recv_shutdown().await;

            info!("shutdown complete");
            ExitCode::Ok
        }
        RunMode::Watch => {
            let mut watcher = FsWatcher::new(watcher, &mut secrets, provider.as_ref());
            match watcher.run().await {
                Ok(()) => ExitCode::Ok,
                Err(e) => {
                    error!(error=%e, "watch errored");
                    ExitCode::IoErr
                }
            }
        }
    }
}
