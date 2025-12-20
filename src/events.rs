//! Signal handling utilities used for graceful shutdown
use async_trait::async_trait;
use futures::future::BoxFuture;
use std::path::PathBuf;
use sysexits::ExitCode;
use tokio::signal::unix::{SignalKind, signal};
use tracing::{debug, info};

/// Filesystem events for SecretFileRegistry
#[derive(Debug, Ord, PartialOrd, Eq, PartialEq)]
pub enum FsEvent {
    Write(PathBuf),
    Remove(PathBuf),
    Move { from: PathBuf, to: PathBuf },
}

/// Handler trait for reacting to locket events
#[async_trait]
pub trait EventHandler: Send + Sync {
    /// Returns the list of file paths this handler monitors.
    ///
    /// Note: Files must exist prior to starting the watcher to be watched successfully.
    /// Non-existent paths will be rejected with WatchError::SourceMissing.
    fn paths(&self) -> Vec<PathBuf>;

    /// Process a batch of coalesced filesystem events which occured within the debounce window.
    async fn handle(&mut self, events: Vec<FsEvent>) -> anyhow::Result<()>;

    /// Returns a future that resolves when the handler wants to exit.
    /// Default: Waits for SIGINT/SIGTERM.
    fn exit_notify(&self) -> BoxFuture<'static, ExitCode> {
        Box::pin(async move {
            wait_for_signal(false).await;
            ExitCode::Ok
        })
    }
    /// Any special handlers needed for resource cleanup should be implemented here.
    /// We cannot cleanup in the exit_notify because we cannot mutably borrow self there.
    /// And we may need to mutably borrow self to cleanup resources.
    async fn cleanup(&mut self) {}
}

/// Listens for shutdown signals.
///
/// when `interactive` is true, ignore SIGINT/SIGQUIT which should be handled by interactive process. Exits only on SIGTERM.
pub async fn wait_for_signal(interactive: bool) {
    // SIGTERM always triggers shutdown
    let mut sigterm = signal(SignalKind::terminate()).expect("failed to install SIGTERM handler");

    if interactive {
        // Ignore TTY signals.
        // The child process shares the TTY and receives these signals directly from the kernel.
        // We must stay alive to manage the child's lifecycle unless explicitly terminated.
        let mut sigint = signal(SignalKind::interrupt()).expect("failed to install SIGINT handler");
        let mut sigquit = signal(SignalKind::quit()).expect("failed to install SIGQUIT handler");

        loop {
            tokio::select! {
                _ = sigterm.recv() => {
                    info!("Received SIGTERM, shutting down...");
                    return;
                }
                _ = sigint.recv() => {
                    debug!("Received SIGINT. Ignored in interactive mode.");
                }
                _ = sigquit.recv() => {
                    debug!("Received SIGQUIT. Ignored in interactive mode.");
                }
            }
        }
    } else {
        // Service Mode: Any termination signal signals a shutdown
        let mut sigint = signal(SignalKind::interrupt()).expect("failed to install SIGINT handler");
        let mut sigquit = signal(SignalKind::quit()).expect("failed to install SIGQUIT handler");

        tokio::select! {
            _ = sigterm.recv() => info!("Received SIGTERM, shutting down..."),
            _ = sigint.recv() => info!("Received SIGINT, shutting down..."),
            _ = sigquit.recv() => info!("Received SIGQUIT, shutting down..."),
        }
    }
}
