//! Signal handling utilities used for graceful shutdown
use tokio::signal::unix::{SignalKind, signal};
use tracing::{debug, info};

/// Listens for shutdown signals.
///
/// when `interactive` is true, ignore SIGINT/SIGQUIT which should be handled by interactive process. Exits only on SIGTERM.
pub async fn recv_shutdown(interactive: bool) {
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
