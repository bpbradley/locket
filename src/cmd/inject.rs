// run.rs
use crate::{
    events,
    health::StatusFile,
    logging::Logger,
    provider::{Provider, ProviderArgs},
    secrets::{SecretFileManager, SecretFileOpts},
    watch::{DebounceDuration, FsWatcher},
};
use clap::{Args, ValueEnum};
use tracing::{debug, error, info};

#[derive(Default, Copy, Clone, Debug, ValueEnum)]
pub enum InjectMode {
    #[default]
    /// **Default** Materialize all secrets once and exit
    OneShot,
    /// **Docker Default** Watch for changes on templates and reinject
    Watch,
    /// Inject once and then park to keep the process alive
    Park,
}

#[derive(Args, Debug)]
pub struct InjectArgs {
    /// Mode of operation
    #[arg(long = "mode", env = "LOCKET_INJECT_MODE", value_enum, default_value_t)]
    pub mode: InjectMode,

    /// Status file path used for healthchecks.
    ///
    /// If not provided, no status file is created.
    ///
    /// **Docker Default:** `/dev/shm/locket/ready`
    #[arg(long = "status-file", env = "LOCKET_STATUS_FILE")]
    pub status_file: Option<StatusFile>,

    /// Secret Management Configuration
    #[command(flatten)]
    pub manager: SecretFileOpts,

    /// Debounce duration for filesystem events in watch mode.
    ///
    /// Events occurring within this duration will be coalesced into a single update
    /// so as to not overwhelm the secrets manager with rapid successive updates from
    /// filesystem noise.
    ///
    /// Handles human-readable strings like "100ms", "2s", etc.
    /// Unitless numbers are interpreted as milliseconds.
    #[arg(long, env = "WATCH_DEBOUNCE", default_value_t = DebounceDuration::default())]
    debounce: DebounceDuration,

    /// Logging configuration
    #[command(flatten)]
    pub logger: Logger,

    /// Secrets provider selection
    #[command(flatten, next_help_heading = "Provider Configuration")]
    provider: ProviderArgs,
}

pub async fn inject(args: InjectArgs) -> Result<(), crate::error::LocketError> {
    args.logger.init()?;
    info!(
        "Starting locket v{} `run` service ",
        env!("CARGO_PKG_VERSION")
    );
    debug!("effective config: {:#?}", args);

    if let Some(status) = &args.status_file {
        debug!("clearing existing status file at startup");
        status.clear().unwrap_or_else(|e| {
            error!(error=%e, "failed to clear status file on startup");
        });
    }

    let provider = Provider::try_from(args.provider)?.build().await?;

    let manager = SecretFileManager::new(args.manager, provider)?;

    manager.inject_all().await?;

    if let Some(status) = &args.status_file {
        debug!("injection complete; creating status file");
        status.mark_ready()?;
    }

    match args.mode {
        InjectMode::OneShot => Ok(()),
        InjectMode::Park => {
            tracing::info!("parking... (ctrl-c to exit)");
            events::wait_for_signal(false).await;

            info!("shutdown complete");
            Ok(())
        }
        InjectMode::Watch => {
            let watcher = FsWatcher::new(args.debounce, manager);
            watcher.run().await?;
            Ok(())
        }
    }
}
