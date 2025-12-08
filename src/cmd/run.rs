// run.rs
use crate::{
    health::StatusFile,
    logging::Logger,
    provider::{Provider, SecretsProvider},
    secrets::{FsEvent, SecretManager, SecretsOpts},
    signal,
    watch::{FsWatcher, WatchError, WatchHandler, WatcherOpts},
};
use async_trait::async_trait;
use clap::{Args, ValueEnum};
use std::path::PathBuf;
use sysexits::ExitCode;
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
    pub manager: SecretsOpts,

    /// Filesystem watcher options
    #[command(flatten)]
    pub watcher: WatcherOpts,

    /// Logging configuration
    #[command(flatten)]
    pub logger: Logger,

    /// Secrets provider selection
    #[command(flatten, next_help_heading = "Provider Configuration")]
    provider: Provider,
}

struct SecretsWatcher<'a> {
    secrets: &'a mut SecretManager,
    provider: &'a dyn SecretsProvider,
}

#[async_trait]
impl<'a> WatchHandler for SecretsWatcher<'a> {
    fn paths(&self) -> Vec<PathBuf> {
        self.secrets
            .options()
            .mapping
            .iter()
            .map(|m| m.src().into())
            .collect()
    }

    async fn handle(&mut self, event: FsEvent) -> Result<(), WatchError> {
        self.secrets
            .handle_fs_event(self.provider, event)
            .await
            .map_err(|e| e.into())
    }
}

pub async fn run(args: RunArgs) -> ExitCode {
    if let Err(e) = args.logger.init() {
        error!(error=%e, "init logging failed");
        return ExitCode::CantCreat;
    }
    info!(
        "Starting locket v{} `run` service ",
        env!("CARGO_PKG_VERSION")
    );
    debug!("effective config: {:#?}", args);

    let RunArgs {
        mut manager,
        status_file,
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

    if let Err(e) = manager.resolve() {
        error!(error=%e, "failed to resolve secret configuration");
        return ExitCode::Config;
    }

    let mut manager = SecretManager::new(manager);

    match manager.collisions() {
        Ok(()) => {}
        Err(e) => {
            error!(error=%e, "secret destination collisions detected");
            return ExitCode::Config;
        }
    };

    if let Err(e) = manager.inject_all(provider.as_ref()).await {
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
            let handler = SecretsWatcher {
                secrets: &mut manager,
                provider: provider.as_ref(),
            };
            let mut watcher = FsWatcher::new(watcher, handler);
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
