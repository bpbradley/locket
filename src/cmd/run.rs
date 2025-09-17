use super::RunArgs;
use crate::{envvars, health, logging, mirror, watch};
use std::collections::HashSet;
use sysexits::ExitCode;
use tracing::{debug, error};

pub fn run(args: RunArgs) -> ExitCode {
    if let Err(e) = logging::init(args.config.log_format, args.config.log_level) {
        error!(error=%e, "init logging failed");
        return ExitCode::CantCreat;
    }
    debug!(?args.config, "effective config");

    let provider = match args.provider.build() {
        Ok(p) => p,
        Err(e) => {
            error!(error=%e, "invalid provider configuration");
            return ExitCode::Config;
        }
    };

    let env_plans = envvars::plan_env_secrets(&args.config);
    let tpl_plans = mirror::plan_templates(&args.config);
    let env_paths: HashSet<_> = env_plans.iter().map(|e| e.dst.clone()).collect();
    let tpl_paths: HashSet<_> = tpl_plans.iter().map(|t| t.dst.clone()).collect();
    let conflicts: Vec<_> = env_paths.intersection(&tpl_paths).cloned().collect();
    if !conflicts.is_empty() {
        error!(?conflicts, "conflict between env secrets and templates");
        return ExitCode::DataErr;
    }

    if let Err(e) = mirror::sync_templates(&args.config, provider.as_ref()) {
        error!(error=%e, "sync templates failed");
        return ExitCode::IoErr;
    }
    if let Err(e) = envvars::sync_env_secrets(&args.config, provider.as_ref()) {
        error!(error=%e, "sync env secrets failed");
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
        match watch::run_watch(&args.config, provider.as_ref()) {
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
