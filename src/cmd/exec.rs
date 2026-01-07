use crate::{
    config::exec::ExecConfig,
    env::EnvManager,
    error::LocketError,
    events::{EventHandler, FsEvent, HandlerError},
    path::AbsolutePath,
    process::ProcessManager,
    secrets::SecretFileManager,
    watch::FsWatcher,
};
use futures::future::BoxFuture;
use std::collections::HashSet;
use tracing::{debug, info};

pub async fn exec(config: ExecConfig) -> Result<(), LocketError> {
    config.logger.init()?;
    info!(
        "Starting locket v{} `exec` service ",
        env!("CARGO_PKG_VERSION")
    );
    debug!("effective config: {:#?}", config);

    // Initialize Provider
    let provider = config.provider.build().await?;

    // Initialize managers / secrets
    let mut env_secrets = config.env_overrides;
    env_secrets.extend(config.env_files);
    let env_manager = EnvManager::new(env_secrets, provider.clone());

    let interactive = config.interactive.unwrap_or(!config.watch);
    let command = config.cmd;
    let mut process = ProcessManager::new(env_manager, command, interactive, config.timeout);

    let files = SecretFileManager::new(config.manager, provider)?;

    // Initial Start
    info!("resolving environment and starting process...");
    files.inject_all().await?;
    process.start().await?;

    let mut handler = ExecOrchestrator::new(process, files);

    // Execution Mode Branch
    if config.watch {
        let watcher = FsWatcher::new(config.debounce, handler);
        // Watcher gives ownership of the handler back when it exits
        // so we can clean up properly.
        handler = watcher.run().await?;
        handler.cleanup().await;
        info!("watch loop terminated gracefully");
        Ok(())
    } else {
        let result = handler.wait().await;
        handler.cleanup().await;
        result.map_err(LocketError::from)
    }
}

struct ExecOrchestrator {
    process: ProcessManager,
    files: SecretFileManager,
    process_paths: HashSet<AbsolutePath>,
}

impl ExecOrchestrator {
    pub fn new(process: ProcessManager, files: SecretFileManager) -> Self {
        let process_paths = process.paths().into_iter().collect();
        Self {
            process,
            files,
            process_paths,
        }
    }
}

#[async_trait::async_trait]
impl EventHandler for ExecOrchestrator {
    fn paths(&self) -> Vec<AbsolutePath> {
        let mut p = self.files.paths();
        p.extend(self.process.paths());
        p
    }

    async fn handle(&mut self, events: Vec<FsEvent>) -> Result<(), HandlerError> {
        let proc_events: Vec<FsEvent> = events
            .iter()
            .filter(|e| e.affects(|p| self.process_paths.contains(p)))
            .cloned()
            .collect();

        // SecretFileManager will ignore paths it does not manage.
        // so it's best to just pass all events
        self.files.handle(events).await?;

        // Handle Process Restarts
        if !proc_events.is_empty() {
            self.process.handle(proc_events).await?;
        }

        Ok(())
    }

    fn wait(&self) -> BoxFuture<'static, Result<(), HandlerError>> {
        // Lifecycle is dictated by the child process, not the files.
        self.process.wait()
    }

    async fn cleanup(&mut self) {
        self.process.cleanup().await;
    }
}
