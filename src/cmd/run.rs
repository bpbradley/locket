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
pub enum RunMode {
    /// Collect and materialize all secrets once and then exit
    OneShot,
    /// Continuously watch for changes on configured templates and update secrets as needed
    #[default]
    Watch,
    /// Run once and then park to keep the process alive
    Park,
}

#[derive(Args, Debug)]
pub struct RunArgs {
    /// Mode of operation
    #[arg(long = "mode", env = "LOCKET_RUN_MODE", value_enum, default_value_t = RunMode::Watch)]
    pub mode: RunMode,

    /// Status file path used for healthchecks
    #[command(flatten)]
    pub status_file: StatusFile,

    /// Secret Management Configuration
    #[command(flatten)]
    pub manager: SecretFileOpts,

    /// Debounce duration for filesystem events in watch mode.
    /// Events occurring within this duration will be coalesced into a single update
    /// so as to not overwhelm the secrets manager with rapid successive updates from
    /// filesystem noise. Handles human-readable strings like "100ms", "2s", etc.
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

pub async fn run(args: RunArgs) -> Result<(), crate::error::LocketError> {
    args.logger.init()?;
    info!(
        "Starting locket v{} `run` service ",
        env!("CARGO_PKG_VERSION")
    );
    debug!("effective config: {:#?}", args);

    let status: &StatusFile = &args.status_file;
    status.clear().unwrap_or_else(|e| {
        error!(error=%e, "failed to clear status file on startup");
    });

    let provider = Provider::from(args.provider).build().await?;

    let manager = SecretFileManager::new(args.manager, provider)?;

    manager.inject_all().await?;

    debug!("injection complete; creating status file");
    status.mark_ready()?;

    match args.mode {
        RunMode::OneShot => Ok(()),
        RunMode::Park => {
            tracing::info!("parking... (ctrl-c to exit)");
            events::wait_for_signal(false).await;

            info!("shutdown complete");
            Ok(())
        }
        RunMode::Watch => {
            let watcher = FsWatcher::new(args.debounce, manager);
            watcher.run().await?;
            Ok(())
        }
    }
}
