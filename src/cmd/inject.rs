use crate::{
    config::inject::{InjectConfig, InjectMode},
    events,
    secrets::SecretFileManager,
    watch::FsWatcher,
};
use tracing::{debug, error, info};

pub async fn inject(config: InjectConfig) -> Result<(), crate::error::LocketError> {
    config.logger.init()?;
    info!(
        "Starting locket v{} `run` service ",
        env!("CARGO_PKG_VERSION")
    );
    debug!("effective config: {:#?}", config);

    if let Some(status) = &config.status_file {
        debug!("clearing existing status file at startup");
        status.clear().unwrap_or_else(|e| {
            error!(error=%e, "failed to clear status file on startup");
        });
    }

    let provider = config.provider.build().await?;

    let manager = SecretFileManager::new(config.manager, provider)?;

    manager.inject_all().await?;

    if let Some(status) = &config.status_file {
        debug!("injection complete; creating status file");
        status.mark_ready()?;
    }

    match config.mode {
        InjectMode::OneShot => Ok(()),
        InjectMode::Park => {
            tracing::info!("parking... (ctrl-c to exit)");
            events::wait_for_signal(false).await;

            info!("shutdown complete");
            Ok(())
        }
        InjectMode::Watch => {
            let watcher = FsWatcher::new(config.debounce, manager);
            watcher.run().await?;
            Ok(())
        }
    }
}
