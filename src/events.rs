//! Core interfaces for event handling.
//!
//! Defines a reactor pattern around a central event loop that:
//! 1. Monitors resources (Filesystem, Signals, etc.).
//! 2. Stabilizes volatile inputs (debouncing , coalescing...).
//! 3. Dispatches actionable events to registered [`EventHandler`]s.
//!
//! This decoupling allows event producers (like a filesystem watcher) to be agnostic about
//! event consumers (like a process manager or template renderer).

#[cfg(any(feature = "exec", feature = "compose"))]
use crate::{env::EnvError, process::ProcessError};
use crate::{provider::ProviderError, secrets::SecretError};
use async_trait::async_trait;
use futures::future::BoxFuture;
use indexmap::IndexMap;
use std::path::PathBuf;
use std::process::{ExitCode, ExitStatus};
use thiserror::Error;
use tokio::signal::unix::{SignalKind, signal};
use tracing::{debug, info};

/// A unified error type for the event loop.
///
/// Serves largely as a control plane for error propagation. It normalizes
/// domain-specific failures into a common format that the
/// event loop can reason about to decide whether to continue, retry, or abort.
#[derive(Debug, Error)]
pub enum HandlerError {
    #[cfg(any(feature = "exec", feature = "compose"))]
    #[error(transparent)]
    Env(#[from] EnvError),

    #[error(transparent)]
    Secret(#[from] SecretError),

    #[error(transparent)]
    Provider(#[from] ProviderError),

    #[cfg(any(feature = "exec", feature = "compose"))]
    #[error(transparent)]
    Process(#[from] ProcessError),

    #[error("Process I/O error")]
    Io(#[from] std::io::Error),

    #[error("Process exited with status {0}")]
    Exited(ExitStatus),

    #[error("Process terminated by signal")]
    Signaled,

    #[error("Operation interrupted")]
    Interrupted,
}

impl HandlerError {
    /// Helper to convert an ExitStatus into a Result
    pub fn from_status(status: ExitStatus) -> Result<(), Self> {
        if status.success() {
            Ok(())
        } else {
            Err(Self::Exited(status))
        }
    }
}

impl From<HandlerError> for ExitCode {
    fn from(e: HandlerError) -> Self {
        match e {
            #[cfg(feature = "exec")]
            HandlerError::Process(proc_err) => proc_err.into(),
            #[cfg(any(feature = "exec", feature = "compose"))]
            HandlerError::Env(_) | HandlerError::Secret(_) => {
                ExitCode::from(sysexits::ExitCode::Config as u8)
            }
            HandlerError::Provider(_) => ExitCode::from(sysexits::ExitCode::Unavailable as u8),
            HandlerError::Io(_) => ExitCode::from(sysexits::ExitCode::IoErr as u8),
            HandlerError::Interrupted => ExitCode::SUCCESS,
            _ => ExitCode::from(sysexits::ExitCode::Software as u8),
        }
    }
}

/// The primary interface for components that participate in the event loop.
///
/// Implementors of this trait are reactors. They define the scope of
/// interest (`paths`) and the logic to execute when the state changes (`handle`).
#[async_trait]
pub trait EventHandler: Send + Sync {
    /// Defines the scope of resources this handler is interested in.
    ///
    /// The event loop uses this to configure the underlying monitors (e.g., `inotify`).
    /// The handler must guarantee that these resources are valid targets for monitoring.
    fn paths(&self) -> Vec<PathBuf>;

    /// Reacts to a batch of events.
    ///
    /// This is the core logic of the reactor. It receives a stable, coalesced list
    /// of changes which must be processed.
    ///
    /// # Errors
    /// * **Ok(()):** The event was handled (successfully or not), and the reactor
    ///   remains in a valid state to process future events. Transient failures
    ///   should be logged internally and return `Ok`.
    /// * **Err(HandlerError):** The reactor has encountered a fatal, unrecoverable
    ///   state. The event loop should terminate.
    async fn handle(&mut self, events: Vec<FsEvent>) -> Result<(), HandlerError>;

    /// A future that resolves when the event loop should exit.
    ///
    /// This allows the reactor to proactively control the application lifecycle,
    /// such as when a managed child process exits or a specific completion condition is met.
    ///
    /// The default implementation waits for OS termination signals (SIGINT/SIGTERM).
    fn wait(&self) -> BoxFuture<'static, Result<(), HandlerError>> {
        Box::pin(async move {
            wait_for_signal(false).await;
            Ok(())
        })
    }

    /// Performs teardown and resource release.
    ///
    /// This hook allows the reactor to perform graceful shutdown operations (e.g.,
    /// sending SIGTERM to children, removing lockfiles) before the application exits.
    async fn cleanup(&mut self) {}
}

/// Represents an actionable change in the monitored environment.
///
/// Currently focused on filesystem changes, as this is the only relevant event source.
/// It is broadly a unit of work for the event loop.
#[derive(Debug, Ord, PartialOrd, Eq, PartialEq)]
pub enum FsEvent {
    /// A resource has been modified or created and is ready for processing.
    Write(PathBuf),
    /// A resource has been removed.
    Remove(PathBuf),
    /// A resource has changed location.
    Move { from: PathBuf, to: PathBuf },
}

/// Registry to collect and coalesce filesystem events.
///
/// It ensures that if a file is written, moved, and then deleted within the
/// processing window, the handler sees only the relevant outcome.
pub struct FsEventRegistry {
    map: IndexMap<PathBuf, FsEvent>,
}

impl FsEventRegistry {
    pub fn new() -> Self {
        Self {
            map: IndexMap::new(),
        }
    }

    /// Update the registry with a new event
    pub fn register(&mut self, event: FsEvent) {
        match event {
            FsEvent::Write(ref path) => {
                self.update(path.clone(), event);
            }
            FsEvent::Remove(ref path) => {
                self.update(path.clone(), event);
            }
            FsEvent::Move { ref from, ref to } => {
                self.handle_move(from.clone(), to.clone());
            }
        }
    }

    /// Handle a move event by resolving it against existing events in the registry
    /// to produce the correct resultant event. This handler attempts to logically resolve the eventual
    /// state of the file after a move, considering prior writes or moves.
    fn handle_move(&mut self, from: PathBuf, to: PathBuf) {
        // Resolve what the event for 'to' should be, based on the state of 'from'.
        let event = match self.map.get(&from) {
            // Write(A) -> Move(A->B) === Write(B).
            Some(FsEvent::Write(_)) => FsEvent::Write(to.clone()),

            // Move(Origin->A) -> Move(A->B) === Move(Origin->B).
            Some(FsEvent::Move { from: origin, .. }) => FsEvent::Move {
                from: origin.clone(),
                to: to.clone(),
            },
            // Just move
            _ => FsEvent::Move {
                from: from.clone(),
                to: to.clone(),
            },
        };

        // Clean up the old path (it no longer exists at that location)
        self.map.shift_remove(&from);

        // Register the new event at the new location
        self.update(to, event);
    }

    /// Update the registry with a new event for a given path, applying coalescing logic
    /// to avoid redundant or conflicting events.
    fn update(&mut self, path: PathBuf, new_event: FsEvent) {
        match (self.map.get(&path), &new_event) {
            //  Write -> Remove === Ignore
            (Some(FsEvent::Write(_)), FsEvent::Remove(_)) => {
                self.map.shift_remove(&path);
            }

            // Move -> Remove === Remove(Origin).
            (Some(FsEvent::Move { .. }), FsEvent::Remove(_)) => {
                self.map.insert(path, new_event);
            }

            // Remove -> Write === Write.
            (Some(FsEvent::Remove(_)), FsEvent::Write(_)) => {
                self.map.insert(path, new_event);
            }

            // Default: The new event overwrites the old state.
            _ => {
                self.map.insert(path, new_event);
            }
        }
    }

    /// Drain all registered events for processing
    pub fn drain(&mut self) -> impl Iterator<Item = FsEvent> + '_ {
        self.map.drain(..).map(|(_, event)| event)
    }

    /// Returns true if no events are pending flush
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

impl Default for FsEventRegistry {
    fn default() -> Self {
        Self::new()
    }
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
