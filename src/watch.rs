//! Filesystem watch: monitor templates dir and re-apply sync on changes

use crate::{config::Config, health, provider::SecretsProvider, secrets::Secrets};
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

pub fn run_watch(
    cfg: &Config,
    secrets: &mut Secrets,
    provider: &dyn SecretsProvider,
) -> anyhow::Result<()> {
    let tpl_dir = Path::new(&cfg.secrets.templates_root);
    if !tpl_dir.exists() {
        std::fs::create_dir_all(tpl_dir)?;
        info!(path=?tpl_dir, "created missing templates directory for watch");
    }

    let (tx, rx) = mpsc::channel::<NotifyResult<Event>>();
    let mut watcher = recommended_watcher(move |res| {
        let _ = tx.send(res);
    })?;
    watcher.watch(tpl_dir, RecursiveMode::Recursive)?;
    info!(path=?tpl_dir, "watching template files for changes");

    // Debounce state
    let debounce = Duration::from_millis(200);
    let mut last_event: Option<Instant> = None;
    let mut pending = false;
    let mut dirty: VecDeque<Event> = VecDeque::new();

    loop {
        match rx.recv_timeout(Duration::from_millis(250)) {
            Ok(Ok(event)) => {
                debug!(?event, "fs event");
                if !is_relevant_event(&event.kind) {
                    continue;
                }
                last_event = Some(Instant::now());
                pending = true;
                dirty.push_back(event);
            }
            Ok(Err(e)) => warn!(error=?e, "watch error"),
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if pending
                    && let Some(t) = last_event
                    && t.elapsed() >= debounce
                {
                    pending = false;
                    let mut ok = 0usize;
                    let mut err = 0usize;

                    // Coalesce and process
                    let mut paths: Vec<PathBuf> = Vec::new();
                    while let Some(ev) = dirty.pop_front() {
                        match ev.kind {
                            // Handle rename with both paths in one shot if available
                            EventKind::Modify(ModifyKind::Name(RenameMode::Both))
                                if ev.paths.len() == 2 =>
                            {
                                let old_src = ev.paths[0].clone();
                                let new_src = ev.paths[1].clone();
                                // Update mapping to new src/dst
                                if secrets.rename_file(old_src.clone(), new_src.clone()) {
                                    match secrets.inject_file(provider, &new_src) {
                                        Ok(true) => ok += 1,
                                        Ok(false) => debug!(
                                            ?new_src,
                                            "rename inject skipped; src not tracked"
                                        ),
                                        Err(e) => {
                                            warn!(error=?e, src=?new_src, "inject error after rename");
                                            err += 1;
                                        }
                                    }
                                } else {
                                    let _ = secrets.remove_file(&old_src);
                                }
                            }
                            _ => {
                                // For all other kinds, handle each path independently later
                                paths.extend(ev.paths.into_iter());
                            }
                        }
                    }

                    paths.sort();
                    paths.dedup();

                    for p in paths {
                        if p.exists() && p.is_file() {
                            if secrets.upsert_file(p.clone()) {
                                match secrets.inject_file(provider, &p) {
                                    Ok(true) => ok += 1,
                                    Ok(false) => debug!(?p, "inject skipped; src not tracked"),
                                    Err(e) => {
                                        warn!(error=?e, src=?p, "inject error");
                                        err += 1;
                                    }
                                }
                            }
                        } else if let Some(dst) = secrets.remove_file(&p) {
                            if let Err(e) = remove_one(&dst) {
                                warn!(error=?e, dst=?dst, "remove error");
                                err += 1;
                            } else {
                                ok += 1;
                            }
                        }
                    }

                    if let Err(e) = health::mark_ready(&cfg.status_file) {
                        warn!(error=?e, "failed to update status file after resync");
                    }
                    info!(ok=?ok, errors=?err, "file watch resync complete");
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                warn!("watcher disconnected; exiting watch loop");
                break;
            }
        }
    }

    Ok(())
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
