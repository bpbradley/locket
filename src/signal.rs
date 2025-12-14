//! Signal handling for graceful shutdown.
//!
//! Handles SIGTERM and SIGINT so that indefinitely running tasks
//! can use them to trigger exit for a graceful shutdown.
use tracing::info;

/// Blocks until a shutdown signal is received.
pub async fn recv_shutdown() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};

        let mut sigterm =
            signal(SignalKind::terminate()).expect("failed to install SIGTERM handler");
        let mut sigint = signal(SignalKind::interrupt()).expect("failed to install SIGINT handler");

        tokio::select! {
            _ = sigterm.recv() => {
                info!("received SIGTERM");
            }
            _ = sigint.recv() => {
                info!("received SIGINT");
            }
        }
    }

    #[cfg(not(unix))]
    {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install CTRL+C handler");
        info!("received ctrl-c");
    }
}
