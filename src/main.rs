use clap::Parser;
use secret_sidecar::{config, envvars, health, logging, mirror, provider, watch};
use secret_sidecar::provider::SecretsProvider;
use tracing::{debug, error};

#[derive(Parser, Debug)]
#[command(name = "secret-sidecar")]
#[command(version, about = "Materialize secrets from environment or templates", long_about = None)]
pub struct Cli {
    /// Run a single sync and exit
    #[arg(long)]
    pub once: bool,

    /// Healthcheck: exit 0 if secrets are ready
    #[arg(long)]
    pub healthcheck: bool,

    /// Configuration overrides
    #[command(flatten)]
    pub config: config::Config,

    #[command(subcommand)]
    pub provider: Option<provider::ProviderSubcommand>,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let cfg = cli.config.clone();

    if cli.healthcheck {
        std::process::exit(if health::is_ready(&cfg.status_file) {
            0
        } else {
            1
        });
    }

    logging::init(&cfg.log_format, &cfg.log_level)?;

    let provider = match cli.provider {
        Some(sc) => sc,
        None => provider::ProviderSubcommand::from_env_or_default()?,
    };

    provider.prepare().map_err(|e| anyhow::anyhow!(e.to_string()))?;

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

    mirror::sync_templates(&cfg, &provider)?;
    envvars::sync_env_secrets(&cfg, &provider)?;

    debug!("initialization complete; creating status file");
    health::mark_ready(&cfg.status_file)?;

    // If --once, exit after one successful sync; otherwise, start watch if enabled
    if cli.once {
        Ok(())
    } else if cfg.watch {
        watch::run_watch(&cfg, &provider)
    } else {
        // Passive block when watch is disabled
        loop {
            std::thread::sleep(std::time::Duration::from_secs(3600));
        }
    }
}
