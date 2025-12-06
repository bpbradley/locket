//! Filesystem watch: monitor templates dir and re-apply sync on changes

use crate::{
    provider::SecretsProvider,
    secrets::{FsEvent, SecretError, SecretManager},
    signal,
};
use clap::Args;
use indexmap::IndexMap;
use notify::{
    Event, RecursiveMode, Result as NotifyResult, Watcher,
    event::{EventKind, ModifyKind, RenameMode},
    recommended_watcher,
};
use std::path::PathBuf;
use std::time::Duration;
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

    #[error("secrets error: {0}")]
    Secret(#[from] SecretError),
}

#[derive(Debug, Clone, Copy, Args)]
pub struct WatcherOpts {
    /// Debounce duration in milliseconds for filesystem events.
    /// Events occurring within this duration will be coalesced into a single update
    /// so as to not overwhelm the secrets manager with rapid successive updates from
    /// filesystem noise.
    #[arg(long, env = "WATCH_DEBOUNCE_MS", default_value_t = 500)]
    debounce_ms: u64,
}

impl Default for WatcherOpts {
    fn default() -> Self {
        Self { debounce_ms: 500 }
    }
}

enum ControlFlow {
    Continue,
    Break,
}

pub struct FsWatcher<'a> {
    secrets: &'a mut SecretManager,
    provider: &'a dyn SecretsProvider,
    debounce: Duration,
    events: EventRegistry,
}

impl<'a> FsWatcher<'a> {
    pub fn new(
        opts: WatcherOpts,
        secrets: &'a mut SecretManager,
        provider: &'a dyn SecretsProvider,
    ) -> Self {
        Self {
            secrets,
            provider,
            debounce: Duration::from_millis(opts.debounce_ms),
            events: EventRegistry::new(),
        }
    }

    pub async fn run(&mut self) -> Result<(), WatchError> {
        let (tx, mut rx) = mpsc::channel::<NotifyResult<Event>>(100);
        let tx_fs = tx.clone();
        let mut watcher = recommended_watcher(move |res| {
            let _ = tx_fs.blocking_send(res);
        })?;
        let shutdown = signal::recv_shutdown();
        tokio::pin!(shutdown);
        for mapping in &self.secrets.options().mapping {
            let watched = mapping.src();
            if !watched.exists() {
                return Err(WatchError::SourceMissing(watched.to_path_buf()));
            }
            let mode = if watched.is_dir() {
                RecursiveMode::Recursive
            } else {
                RecursiveMode::NonRecursive
            };
            watcher.watch(watched, mode)?;
            info!(path=?watched, "watching for changes");
        }

        loop {
            debug!("waiting for fs event");

            let event = tokio::select! {
                _ = &mut shutdown => {
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

    fn ingest_event(&mut self, event: Event) -> bool {
        if let Some(fs_ev) = Self::map_fs_event(&event) {
            self.events.register(fs_ev);
            return true;
        }
        false
    }

    async fn flush_events(&mut self) {
        if self.events.is_empty() {
            return;
        }

        let events: Vec<_> = self.events.drain().collect();
        debug!(count = events.len(), "processing batched fs events");

        for ev in events {
            if let Err(e) = self.secrets.handle_fs_event(self.provider, ev).await {
                warn!(error=?e, "failed to handle fs event");
            }
        }
    }

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

/// Registry to collect and coalesce filesystem events before processing in batch
pub struct EventRegistry {
    map: IndexMap<PathBuf, FsEvent>,
}

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

    pub fn drain(&mut self) -> impl Iterator<Item = FsEvent> + '_ {
        self.map.drain(..).map(|(_, event)| event)
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

impl Default for EventRegistry {
    fn default() -> Self {
        Self::new()
    }
}
