mod config;
mod envvars;
mod health;
mod logging;
mod mirror;
mod provider;
mod watch;
mod write;

use clap::{ArgAction, Parser};
use tracing::{debug, error};

#[derive(Parser, Debug)]
#[command(name = "secret-sidecar")]
#[command(version, about = "Materialize secrets from environment or templates", long_about = None)]
struct Cli {
    /// Run a single sync then block (no watch yet)
    #[arg(long, action=ArgAction::SetTrue)]
    once: bool,

    /// Healthcheck path
    #[arg(long, value_name = "PATH")]
    healthcheck: Option<String>,

    /// Log format: text|json
    #[arg(long, value_name="FORMAT", default_value_t=String::from("text"))]
    log_format: String,

    /// Log level: trace|debug|info|warn|error
    #[arg(long, value_name="LEVEL", default_value_t=String::from("info"))]
    log_level: String,

    /// Templates directory (overrides TEMPLATES_DIR)
    #[arg(long, value_name = "PATH")]
    templates_dir: Option<String>,

    /// Output directory (overrides OUTPUT_DIR)
    #[arg(long, value_name = "PATH")]
    output_dir: Option<String>,

    /// Status file path (overrides STATUS_FILE)
    #[arg(long, value_name = "PATH")]
    status_file: Option<String>,

    /// Watch for changes (overrides WATCH)
    #[arg(long, value_name="BOOL", value_parser=clap::value_parser!(bool))]
    watch: Option<bool>,

    /// Allow inject fallback to raw copy (overrides INJECT_FALLBACK_COPY)
    #[arg(long, value_name="BOOL", value_parser=clap::value_parser!(bool))]
    inject_fallback_copy: Option<bool>,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    if let Some(path) = &cli.healthcheck {
        std::process::exit(if health::is_ready(path) { 0 } else { 1 });
    }

    logging::init(&cli.log_format, &cli.log_level)?;
    let mut cfg = config::Config::from_env()?;
    // CLI overrides env
    if let Some(v) = cli.templates_dir.clone() {
        cfg.templates_dir = v;
    }
    if let Some(v) = cli.output_dir.clone() {
        cfg.output_dir = v;
    }
    if let Some(v) = cli.status_file.clone() {
        cfg.status_file = v;
    }
    if let Some(v) = cli.watch {
        cfg.watch = v;
    }
    if let Some(v) = cli.inject_fallback_copy {
        cfg.inject_fallback_copy = v;
    }
    let provider = provider::build_provider(&cfg).map_err(|e| anyhow::anyhow!(e.to_string()))?;
    if let Err(e) = provider.prepare() {
        error!("{}", e);
        std::process::exit(1);
    }

    // Plan both sides for conflict detection
    let env_plans = envvars::plan_env_secrets(&cfg);
    let tpl_plans = mirror::plan_templates(&cfg);
    use std::collections::HashSet;
    let env_paths: HashSet<_> = env_plans.iter().map(|e| e.dst.clone()).collect();
    let tpl_paths: HashSet<_> = tpl_plans.iter().map(|t| t.dst.clone()).collect();
    let conflicts: Vec<_> = env_paths.intersection(&tpl_paths).cloned().collect();
    if !conflicts.is_empty() {
        error!(?conflicts, "conflict between env secrets and templates");
        std::process::exit(2);
    }

    mirror::sync_templates(&cfg, provider.as_ref())?;
    envvars::sync_env_secrets(&cfg, provider.as_ref())?;

    debug!("initialization complete; creating status file");
    health::mark_ready(&cfg.status_file)?;

    // If --once, exit after one successful sync; otherwise, start watch if enabled
    if cli.once {
        Ok(())
    } else if cfg.watch {
        watch::run_watch(&cfg, provider.as_ref())
    } else {
        // Passive block when watch is disabled
        loop {
            std::thread::sleep(std::time::Duration::from_secs(3600));
        }
    }
}
