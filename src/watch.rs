//! Filesystem watch: monitor templates dir and re-apply sync on changes.
//!
//! Internally handles the complexity of "debouncing" (waiting for a quiet period)
//! and "coalescing" (merging multiple rapid events into a single logical operation)
//! to prevent the secret manager from thrashing.

use async_trait::async_trait;
use indexmap::IndexMap;
use notify::{
    Event, RecursiveMode, Result as NotifyResult, Watcher,
    event::{EventKind, ModifyKind, RenameMode},
    recommended_watcher,
};
use std::time::Duration;
use std::{path::PathBuf, str::FromStr};
use thiserror::Error;
use tokio::sync::mpsc;
use tokio::time::{self, Instant};
use tracing::{debug, info, warn};
mod file;
#[cfg(feature = "exec")]
mod process;
pub use file::FileHandler;
#[cfg(feature = "exec")]
pub use process::ProcessHandler;

#[derive(Debug, Error)]
pub enum WatchError {
    #[error("filesystem watcher disconnected unexpectedly")]
    Disconnected,

    #[error("notify error: {0}")]
    Notify(#[from] notify::Error),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("source path missing: {0}")]
    SourceMissing(PathBuf),
}

/// Filesystem events for SecretFileRegistry
#[derive(Debug, Ord, PartialOrd, Eq, PartialEq)]
pub enum FsEvent {
    Write(PathBuf),
    Remove(PathBuf),
    Move { from: PathBuf, to: PathBuf },
}

/// Handler trait for reacting to filesystem events
/// Implementors define what paths to watch and how to handle events on those paths.
#[async_trait]
pub trait WatchHandler: Send + Sync {
    /// Return the list of paths to monitor.
    /// Files must exist prior to starting the watcher.
    fn paths(&self) -> Vec<PathBuf>;

    /// Handle filesystem event dispatched by the watcher.
    async fn handle(&mut self, events: Vec<FsEvent>) -> anyhow::Result<()>;
}

enum ControlFlow {
    Continue,
    Break,
}

/// Filesystem watcher that monitors specified paths and dispatches events to a handler.
///
/// This struct owns the `notify` watcher and manages the lifecycle of event
/// collection, debouncing, and flushing.
pub struct FsWatcher<H: WatchHandler> {
    handler: H,
    debounce: Duration,
    events: EventRegistry,
}

impl<H: WatchHandler> FsWatcher<H> {
    /// Create a new FsWatcher.
    ///
    /// * `debounce`: The quiet period required before flushing events.
    /// * `handler`: Operational logic to execute on filesystem events.
    pub fn new(debounce: impl Into<Duration>, handler: H) -> Self {
        Self {
            handler,
            debounce: debounce.into(),
            events: EventRegistry::new(),
        }
    }

    /// Run the watcher loop until `shutdown` resolves.
    ///
    /// This function blocks the current task. It will:
    /// 1. Register watches on all paths provided by the handler.
    /// 2. Buffer incoming filesystem events.
    /// 3. Debounce events
    /// 4. Flush coalesced events to `handler.handle()` when debounce period expires
    pub async fn run<F>(&mut self, shutdown: F) -> Result<(), WatchError>
    where
        F: Future<Output = ()> + Send + 'static,
    {
        let (tx, mut rx) = mpsc::channel::<NotifyResult<Event>>(100);
        let tx_fs = tx.clone();
        let mut watcher = recommended_watcher(move |res| {
            let _ = tx_fs.blocking_send(res);
        })?;
        tokio::pin!(shutdown);
        for watched in self.handler.paths() {
            if !watched.exists() {
                return Err(WatchError::SourceMissing(watched.to_path_buf()));
            }
            let mode = if watched.is_dir() {
                RecursiveMode::Recursive
            } else {
                RecursiveMode::NonRecursive
            };
            watcher.watch(&watched, mode)?;
            info!(path=?watched, "watching for changes");
        }

        loop {
            debug!("waiting for fs event");

            let event = tokio::select! {
                _ = shutdown.as_mut() => {
                    info!("shutdown signal received; exiting watcher");
                    return Ok(());
                }
                signal = rx.recv() => {
                    match signal {
                        Some(Ok(ev)) => ev,
                        Some(Err(e)) => {
                            warn!(error=?e, "notify internal error");
                            continue;
                        }
                        None => return Err(WatchError::Disconnected),
                    }
                }
            };

            if !self.ingest_event(event) {
                continue;
            }

            // Enter debounce loop to coalesce rapid successive events
            match self.debounce_loop(&mut rx, &mut shutdown).await? {
                ControlFlow::Continue => {
                    self.flush_events().await;
                }
                ControlFlow::Break => {
                    info!("exiting watcher loop.");
                    return Ok(());
                }
            }
        }
    }

    /// Debounce loop to wait for a quiet period before processing events so as not to overwhelm the handler
    async fn debounce_loop<F>(
        &mut self,
        rx: &mut mpsc::Receiver<NotifyResult<Event>>,
        shutdown: &mut std::pin::Pin<&mut F>,
    ) -> Result<ControlFlow, WatchError>
    where
        F: Future<Output = ()>,
    {
        debug!("Debouncing events for {:?}", self.debounce);
        let deadline = Instant::now() + self.debounce;

        // Use sleep_until for precise deadline handling
        let sleep = time::sleep_until(deadline);
        tokio::pin!(sleep);

        loop {
            tokio::select! {
                // Timeout reached. No new events in debounce period.
                _ = &mut sleep => {
                    return Ok(ControlFlow::Continue);
                }

                _ = shutdown.as_mut() => {
                    info!("shutdown signal received");
                    return Ok(ControlFlow::Break);
                }
                // New event received before timeout.
                signal = rx.recv() => {
                    match signal {
                        Some(Ok(event)) => {
                            if self.ingest_event(event) {
                                // Reset the timer
                                sleep.as_mut().reset(Instant::now() + self.debounce);
                            }
                        }
                        Some(Err(e)) => {
                            warn!(error=?e, "notify internal error");
                        }
                        None => return Err(WatchError::Disconnected),
                    }
                }
            }
        }
    }

    /// Ingest a filesystem event into the registry, returning true if it was relevant
    fn ingest_event(&mut self, event: Event) -> bool {
        if let Some(fs_ev) = Self::map_fs_event(&event) {
            self.events.register(fs_ev);
            return true;
        }
        false
    }

    /// Flush the registered events to the handler for processing
    async fn flush_events(&mut self) {
        if self.events.is_empty() {
            return;
        }

        let events: Vec<_> = self.events.drain().collect();
        debug!(count = events.len(), "processing batched fs events");

        if let Err(e) = self.handler.handle(events).await {
            warn!(error=?e, "failed to handle fs events");
        }
    }

    /// Map a notify Event to an FsEvent, if relevant
    fn map_fs_event(event: &Event) -> Option<FsEvent> {
        match &event.kind {
            EventKind::Create(_) | EventKind::Modify(ModifyKind::Data(_)) => {
                event.paths.first().map(|src| FsEvent::Write(src.clone()))
            }
            EventKind::Remove(_) => event.paths.first().map(|src| FsEvent::Remove(src.clone())),
            EventKind::Modify(ModifyKind::Name(mode)) => match mode {
                RenameMode::Both => {
                    if event.paths.len() == 2 {
                        Some(FsEvent::Move {
                            from: event.paths[0].clone(),
                            to: event.paths[1].clone(),
                        })
                    } else {
                        None
                    }
                }
                // Renamed to an unknown location == Remove(X)
                RenameMode::From => event.paths.first().map(|src| FsEvent::Remove(src.clone())),
                // Renamed from an unknown location == Write(X)
                RenameMode::To => event.paths.first().map(|src| FsEvent::Write(src.clone())),
                _ => None,
            },
            _ => None,
        }
    }
}

/// Registry to collect and coalesce filesystem events.
///
/// It ensures that if a file is written, moved, and then deleted within the
/// debounce window, the handler sees only the relevant outcome.
pub struct EventRegistry {
    map: IndexMap<PathBuf, FsEvent>,
}

/// Implementation of the EventRegistry
impl EventRegistry {
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

impl Default for EventRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Debounce duration wrapper to support human-readable parsing and sane defaults for watcher
#[derive(Debug, Clone, Copy)]
pub struct DebounceDuration(pub Duration);

/// Defaults to milliseconds if no unit specified, otherwise uses humantime parsing.
impl FromStr for DebounceDuration {
    type Err = humantime::DurationError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        /* Defaults to millseconds if no unit specified */
        if let Ok(ms) = s.parse::<u64>() {
            return Ok(DebounceDuration(Duration::from_millis(ms)));
        }
        let duration = humantime::parse_duration(s)?;
        Ok(DebounceDuration(duration))
    }
}

impl std::fmt::Display for DebounceDuration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", humantime::format_duration(self.0))
    }
}

impl From<DebounceDuration> for Duration {
    fn from(val: DebounceDuration) -> Self {
        val.0
    }
}

impl Default for DebounceDuration {
    fn default() -> Self {
        DebounceDuration(Duration::from_millis(500))
    }
}
