//! Filesystem watch: monitor templates dir and re-apply sync on changes

use crate::{provider::SecretsProvider, secrets::Secrets};
use notify::{
    Event, RecursiveMode, Result as NotifyResult, Watcher,
    event::{EventKind, ModifyKind, RenameMode},
    recommended_watcher,
};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};
use tracing::{debug, info, warn};

pub struct FsWatcher<'a> {
    secrets: &'a mut Secrets,
    provider: &'a dyn SecretsProvider,
    debounce: Duration,
    last_event: Option<Instant>,
    pending: bool,
    dirty: VecDeque<Event>,
}

impl<'a> FsWatcher<'a> {
    pub fn new(secrets: &'a mut Secrets, provider: &'a dyn SecretsProvider) -> Self {
        Self {
            secrets,
            provider,
            debounce: Duration::from_millis(200),
            last_event: None,
            pending: false,
            dirty: VecDeque::new(),
        }
    }

    pub fn run(&mut self) -> anyhow::Result<()> {
        let tpl_dir = std::path::Path::new(&self.secrets.options().templates_root);
        std::fs::create_dir_all(tpl_dir)?;

        let (tx, rx) = mpsc::channel::<NotifyResult<Event>>();
        let mut watcher = recommended_watcher(move |res| {
            let _ = tx.send(res);
        })?;
        watcher.watch(tpl_dir, RecursiveMode::Recursive)?;
        info!(path=?tpl_dir, "watching template files for changes");

        loop {
            match rx.recv_timeout(Duration::from_millis(250)) {
                Ok(Ok(event)) => self.handle_event(event),
                Ok(Err(e)) => warn!(error=?e, "watch error"),
                Err(mpsc::RecvTimeoutError::Timeout) => self.maybe_flush(),
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    warn!("watcher disconnected unexpectedly; terminating");
                    return Err(anyhow::anyhow!("filesystem watcher disconnected"));
                }
            }
        }
    }

    fn handle_event(&mut self, event: Event) {
        debug!(?event, "fs event");
        if !is_relevant_event(&event.kind) {
            return;
        }

        self.last_event = Some(Instant::now());
        self.pending = true;
        self.dirty.push_back(event);
    }

    fn maybe_flush(&mut self) {
        if !self.pending {
            return;
        }

        if let Some(t) = self.last_event {
            if t.elapsed() >= self.debounce {
                self.pending = false;
                self.process_pending();
            }
        }
    }

    fn process_pending(&mut self) {
        let mut paths: Vec<PathBuf> = Vec::new();

        while let Some(ev) = self.dirty.pop_front() {
            match ev.kind {
                EventKind::Modify(ModifyKind::Name(RenameMode::Both)) if ev.paths.len() == 2 => {
                    let old_src = ev.paths[0].clone();
                    let new_src = ev.paths[1].clone();
                    if self.secrets.rename_file(old_src.clone(), new_src.clone()) {
                        self.inject_with_logging(&new_src);
                    } else {
                        let _ = self.secrets.remove_file(&old_src);
                    }
                }
                _ => {
                    paths.extend(ev.paths.into_iter());
                }
            }
        }

        paths.sort_unstable();
        paths.dedup();

        for p in paths {
            if p.exists() && p.is_file() {
                self.inject_with_logging(&p);
            } else if let Some(dst) = self.secrets.remove_file(&p)
                && let Err(e) = remove_one(&dst)
            {
                warn!(error=?e, dst=?dst, "remove error");
            }
        }
    }

    fn inject_with_logging(&mut self, p: &Path) {
        if self.secrets.upsert_file(p.to_path_buf()) {
            match self.secrets.inject_file(self.provider, p) {
                Ok(true) => {}
                Ok(false) => debug!(?p, "inject skipped; src not tracked"),
                Err(e) => warn!(error=?e, src=?p, "inject error"),
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

fn remove_one(dst: &Path) -> anyhow::Result<()> {
    if dst.exists() && dst.is_file() {
        std::fs::remove_file(dst)?;
    }
    Ok(())
}
