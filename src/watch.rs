//! Filesystem watch: monitor templates dir and re-apply sync on changes

use crate::{
    provider::SecretsProvider,
    secrets::{FsEvent, Secrets},
};
use notify::{
    Event, RecursiveMode, Result as NotifyResult, Watcher,
    event::{EventKind, ModifyKind, RenameMode},
    recommended_watcher,
};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};
use thiserror::Error;
use tracing::{debug, info, warn};

#[derive(Debug, Error)]
pub enum WatchError {
    #[error("filesystem watcher disconnected unexpectedly")]
    Disconnected,

    #[error("notify error: {0}")]
    Notify(#[from] notify::Error),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

pub struct FsWatcher<'a> {
    secrets: &'a mut Secrets,
    provider: &'a dyn SecretsProvider,
    debounce: Duration,
    dirty: VecDeque<Event>,
}

impl<'a> FsWatcher<'a> {
    pub fn new(secrets: &'a mut Secrets, provider: &'a dyn SecretsProvider) -> Self {
        Self {
            secrets,
            provider,
            debounce: Duration::from_millis(200),
            dirty: VecDeque::new(),
        }
    }

    pub fn run(&mut self) -> Result<(), WatchError> {
        let tpl_dir = std::path::Path::new(&self.secrets.options().templates_root);
        std::fs::create_dir_all(tpl_dir)?;

        let (tx, rx) = mpsc::channel::<NotifyResult<Event>>();
        let mut watcher = recommended_watcher(move |res| {
            let _ = tx.send(res);
        })?;
        watcher.watch(tpl_dir, RecursiveMode::Recursive)?;
        info!(path=?tpl_dir, "watching template files for changes");
        let mut timer = DebounceTimer::new(self.debounce);

        loop {
            if !timer.armed() {
                // Nothing is pending, as the timer isn't armed. We can block indefinitely.
                match rx.recv() {
                    Ok(Ok(event)) => {
                        self.handle_event(event);
                        timer.schedule();
                    }
                    Ok(Err(e)) => warn!(error=?e, "watch error"),
                    Err(mpsc::RecvError) => {
                        warn!("watcher disconnected unexpectedly; terminating");
                        return Err(WatchError::Disconnected);
                    }
                }
                // If something is now pending, continue to the next iteration to handle with debounce.
                continue;
            }

            if timer.try_fire() {
                // Debounce timer fired. Process pending events.
                self.process_pending();
                continue;
            }

            let remaining = timer.remaining().unwrap_or(Duration::ZERO);

            match rx.recv_timeout(remaining) {
                Ok(Ok(event)) => {
                    self.handle_event(event);
                    timer.schedule(); // Reschedule work
                }
                Ok(Err(e)) => warn!(error=?e, "watch error"),
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    timer.cancel();
                    self.process_pending();
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    warn!("watcher disconnected unexpectedly; terminating");
                    return Err(WatchError::Disconnected);
                }
            }
        }
    }

    fn handle_event(&mut self, event: Event) {
        debug!(?event, "fs event");
        if !Self::is_relevant_event(&event.kind) {
            return;
        }
        self.dirty.push_back(event);
    }

    fn process_pending(&mut self) {
        let mut paths: Vec<PathBuf> = Vec::new();

        while let Some(ev) = self.dirty.pop_front() {
            match ev.kind {
                EventKind::Modify(ModifyKind::Name(RenameMode::Both)) if ev.paths.len() == 2 => {
                    let watch_ev = FsEvent::Renamed {
                        old: &ev.paths[0],
                        new: &ev.paths[1],
                    };

                    if let Err(e) = self.secrets.handle_fs_event(self.provider, watch_ev) {
                        warn!(error=?e, old=?ev.paths[0], new=?ev.paths[1], "rename handling error");
                    }
                }
                _ => {
                    paths.extend(ev.paths.into_iter());
                }
            }
        }

        // Deduplicate paths because multiple events may have occurred on the same file.
        // Must sort first to use dedup, but relative order doesn't matter so sort_unstable is fine.
        paths.sort_unstable();
        paths.dedup();

        for p in paths {
            if p.exists() && p.is_file() {
                let ev = FsEvent::CreatedOrModified { src: &p };
                if let Err(e) = self.secrets.handle_fs_event(self.provider, ev) {
                    warn!(error=?e, "failed to handle fs created/modified event");
                }
            } else {
                let ev = FsEvent::Removed { src: &p };
                if let Err(e) = self.secrets.handle_fs_event(self.provider, ev) {
                    warn!(error=?e, "failed to handle fs removed event");
                }
            }
        }
    }

    #[inline]
    fn is_relevant_event(kind: &EventKind) -> bool {
        use EventKind as EK;
        use ModifyKind as MK;
        matches!(
            kind,
            EK::Create(_)
                | EK::Remove(_)
                | EK::Modify(MK::Data(_))
                | EK::Modify(MK::Name(_))
                | EK::Modify(MK::Any)
        )
    }
}

struct DebounceTimer {
    deadline: Option<Instant>,
    period: Duration,
}

impl DebounceTimer {
    fn new(period: Duration) -> Self {
        Self {
            deadline: None,
            period,
        }
    }

    fn schedule(&mut self) {
        self.deadline = Some(Instant::now() + self.period);
    }

    fn cancel(&mut self) {
        self.deadline = None;
    }

    fn remaining(&self) -> Option<Duration> {
        self.deadline.map(|d| {
            let now = Instant::now();
            if d <= now { Duration::ZERO } else { d - now }
        })
    }

    fn armed(&self) -> bool {
        self.deadline.is_some()
    }

    fn try_fire(&mut self) -> bool {
        if let Some(d) = self.deadline
            && Instant::now() >= d
        {
            self.deadline = None;
            return true;
        }
        false
    }
}
