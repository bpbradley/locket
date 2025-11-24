//! Filesystem watch: monitor templates dir and re-apply sync on changes

use crate::{
    provider::SecretsProvider,
    secrets::{Secrets, FsEvent},
};
use indexmap::IndexMap;
use notify::{
    Event, RecursiveMode, Result as NotifyResult, Watcher,
    event::{EventKind, ModifyKind, RenameMode},
    recommended_watcher,
};
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};
use thiserror::Error;
use tracing::{debug, warn};

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

pub struct FsWatcher<'a> {
    secrets: &'a mut Secrets,
    provider: &'a dyn SecretsProvider,
    debounce: Duration,
    events: EventRegistry,
}

impl<'a> FsWatcher<'a> {
    pub fn new(secrets: &'a mut Secrets, provider: &'a dyn SecretsProvider) -> Self {
        Self {
            secrets,
            provider,
            debounce: Duration::from_millis(500),
            events: EventRegistry::new(),
        }
    }

    pub fn run(&mut self) -> Result<(), WatchError> {
        let (tx, rx) = mpsc::channel::<NotifyResult<Event>>();
        let mut watcher = recommended_watcher(move |res| {
            let _ = tx.send(res);
        })?;
        for mapping in &self.secrets.options().mapping {
            let watched = &mapping.src();
            if !watched.exists() {
                return Err(WatchError::SourceMissing(watched.to_path_buf()));
            }
            let mode = if watched.is_dir() {
                RecursiveMode::Recursive
            } else {
                RecursiveMode::NonRecursive
            };
            watcher.watch(watched, mode)?;
        }

        loop {
            debug!("waiting for fs event indefinitely");
            let event = match rx.recv() {
                Ok(Ok(event)) => event,
                Ok(Err(e)) => {
                    warn!(error=?e, "notify internal error");
                    continue;
                }
                Err(_) => return Err(WatchError::Disconnected),
            };

            if !self.ingest_event(event) {
                continue;
            }

            debug!("Debouncing events for {:?}", self.debounce);
            let mut deadline = Instant::now() + self.debounce;
            loop {
                let now = Instant::now();
                if now >= deadline {
                    break;
                }

                let timeout = deadline - now;
                match rx.recv_timeout(timeout) {
                    Ok(Ok(event)) => {
                        // Only extend the window if the event is relevant
                        if self.ingest_event(event) {
                            deadline = Instant::now() + self.debounce;
                        }
                    }
                    Ok(Err(e)) => warn!(error=?e, "notify internal error"),
                    Err(mpsc::RecvTimeoutError::Timeout) => break, // Time is up
                    Err(mpsc::RecvTimeoutError::Disconnected) => {
                        warn!("watcher disconnected unexpectedly");
                        return Err(WatchError::Disconnected);
                    }
                }
            }
            self.flush_events();
        }
    }

    fn ingest_event(&mut self, event: Event) -> bool {
        if let Some(fs_ev) = Self::map_fs_event(&event) {
            self.events.register(fs_ev);
            return true;
        }
        false
    }

    fn flush_events(&mut self) {
        if self.events.is_empty() {
            return;
        }

        let events: Vec<_> = self.events.drain().collect();
        debug!(count = events.len(), "processing batched fs events");

        for ev in events {
            if let Err(e) = self.secrets.handle_fs_event(self.provider, ev) {
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

            // Case 3: File was stable (not in map) or previously removed (rare edge case).
            // Logic: Standard Move optimization applies.
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
