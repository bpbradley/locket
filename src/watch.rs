//! Filesystem watching with debouncing and event coalescing.
//!
//! This module providers a generic filesystem watcher, which can be used in various contexts.
//! It handles the complexity of "debouncing" (waiting for a quiet period)
//! and "coalescing" (merging multiple rapid events, like Create+Modify)
//! to prevent the secret manager from thrashing or performing redundant updates.
//! Implementers provide a `EventHandler` trait to specify which paths to watch
//! and how to handle the resulting events.

use crate::events::{EventHandler, FsEvent, FsEventRegistry};
use crate::path::AbsolutePath;
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

    #[error(transparent)]
    Handler(#[from] crate::events::HandlerError),
}

enum ControlFlow {
    Continue,
    Break,
}

/// A Filesystem watcher that manages the lifecycle of event collection, debouncing, and flushing.
pub struct FsWatcher<H: EventHandler> {
    handler: H,
    debounce: Duration,
    events: FsEventRegistry,
}

impl<H: EventHandler> FsWatcher<H> {
    /// Create a new FsWatcher.
    ///
    /// * `debounce`: The quiet period required before flushing events.
    /// * `handler`: The logic to execute when events occur.
    pub fn new(debounce: impl Into<Duration>, handler: H) -> Self {
        Self {
            handler,
            debounce: debounce.into(),
            events: FsEventRegistry::new(),
        }
    }

    /// Run the watcher loop until `shutdown` resolves.
    ///
    /// This blocks the current task. It will:
    /// 1. Register watches on all paths provided by the handler.
    /// 2. Buffer incoming events.
    /// 3. Wait for the `debounce` duration to pass without new events.
    /// 4. Flush the coalesced events to `handler.handle()`.
    ///
    /// # Errors
    /// Returns `WatchError` if the underlying `notify` watcher fails or paths are missing.
    pub async fn run(mut self) -> Result<H, WatchError> {
        let (tx, mut rx) = mpsc::channel::<NotifyResult<Event>>(100);
        let tx_fs = tx.clone();
        let mut watcher = recommended_watcher(move |res| {
            let _ = tx_fs.blocking_send(res);
        })?;
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
            let exit = self.handler.wait();

            let event = tokio::select! {
                res = exit => {
                    match res {
                        Ok(_) => {
                            info!("handler exit signal received");
                            break;
                        }
                        Err(e) => return Err(WatchError::Handler(e)),
                    }
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
            match self.debounce_loop(&mut rx).await? {
                ControlFlow::Continue => {
                    self.flush_events().await?;
                }
                ControlFlow::Break => {
                    info!("exiting watcher loop.");
                    break;
                }
            }
        }
        Ok(self.handler)
    }

    /// Debounce loop to wait for a quiet period before processing events so as not to overwhelm the handler
    async fn debounce_loop(
        &mut self,
        rx: &mut mpsc::Receiver<NotifyResult<Event>>,
    ) -> Result<ControlFlow, WatchError> {
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
                res = self.handler.wait() => {
                    match res {
                        Ok(_) => {
                            info!("handler exit signal received during debounce.");
                            return Ok(ControlFlow::Break);
                        }
                        Err(e) => return Err(WatchError::Handler(e)),
                    }
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
    async fn flush_events(&mut self) -> Result<(), WatchError> {
        if self.events.is_empty() {
            return Ok(());
        }

        let events: Vec<_> = self.events.drain().collect();
        debug!(count = events.len(), "processing batched fs events");

        self.handler.handle(events).await?;
        Ok(())
    }

    /// Map a notify Event to an FsEvent, if relevant
    fn map_fs_event(event: &Event) -> Option<FsEvent> {
        match &event.kind {
            EventKind::Create(_) | EventKind::Modify(ModifyKind::Data(_)) => event
                .paths
                .first()
                .map(|src| FsEvent::Write(src.into())),
            EventKind::Remove(_) => event
                .paths
                .first()
                .map(|src| FsEvent::Remove(src.into())),
            EventKind::Modify(ModifyKind::Name(mode)) => match mode {
                RenameMode::Both => {
                    if let [from, to, ..] = &event.paths[..] {
                        Some(FsEvent::Move {
                            from: from.into(),
                            to: to.into(),
                        })
                    } else {
                        None
                    }
                }
                // Renamed to an unknown location == Remove(X)
                RenameMode::From => event
                    .paths
                    .first()
                    .map(|src| FsEvent::Remove(src.into())),
                // Renamed from an unknown location == Write(X)
                RenameMode::To => event
                    .paths
                    .first()
                    .map(|src| FsEvent::Write(src.into())),
                _ => None,
            },
            _ => None,
        }
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
