use super::RunArgs;
use crate::{envvars, health, logging, mirror, watch};
use std::collections::HashSet;
use tracing::{debug, error};

pub fn run(args: RunArgs) -> anyhow::Result<()> {
    logging::init(args.config.log_format, args.config.log_level)?;
    debug!(?args.config, "effective config");

    let provider = args.provider.build()?;

    let env_plans = envvars::plan_env_secrets(&args.config);
    let tpl_plans = mirror::plan_templates(&args.config);
    let env_paths: HashSet<_> = env_plans.iter().map(|e| e.dst.clone()).collect();
    let tpl_paths: HashSet<_> = tpl_plans.iter().map(|t| t.dst.clone()).collect();
    let conflicts: Vec<_> = env_paths.intersection(&tpl_paths).cloned().collect();
    if !conflicts.is_empty() {
        error!(?conflicts, "conflict between env secrets and templates");
        std::process::exit(2);
    }

    mirror::sync_templates(&args.config, provider.as_ref())?;
    envvars::sync_env_secrets(&args.config, provider.as_ref())?;

    debug!("initialization complete; creating status file");
    health::mark_ready(&args.config.status_file)?;

    if args.once {
        Ok(())
    } else if args.config.watch {
        watch::run_watch(&args.config, provider.as_ref())
    } else {
        std::thread::park();
        Ok(())
    }
}
