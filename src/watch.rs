//! Filesystem watch: monitor templates dir and re-apply sync on changes

use crate::{config::Config, health, mirror, provider::SecretsProvider};
use notify::{recommended_watcher, Event, RecursiveMode, Result as NotifyResult, Watcher};
use std::path::Path;
use std::sync::mpsc;
use std::time::{Duration, Instant};
use tracing::{debug, info, warn};

/// Start watching the templates directory. On any change, determine the source
/// and selectively sync those changes to the output.
pub fn run_watch(cfg: &Config, provider: &dyn SecretsProvider) -> anyhow::Result<()> {
    // Ensure the templates directory exists so the watcher can attach.
    let tpl_dir = Path::new(&cfg.templates_dir);
    if !tpl_dir.exists() {
        std::fs::create_dir_all(tpl_dir)?;
        info!(path=?tpl_dir, "created missing templates directory for watch");
    }

    let (tx, rx) = mpsc::channel::<NotifyResult<Event>>();
    let mut watcher = recommended_watcher(move |res| {
        // Send results to the receiver loop; ignore send errors if the loop ended.
        let _ = tx.send(res);
    })?;
    watcher.watch(tpl_dir, RecursiveMode::Recursive)?;

    info!(path=?tpl_dir, "watching templates for changes");

    // Simple debounce: wait for a quiet period before syncing
    let debounce = Duration::from_millis(200);
    let mut last_event: Option<Instant> = None;
    let mut pending = false;
    let mut dirty_paths: Vec<std::path::PathBuf> = Vec::new();

    loop {
        match rx.recv_timeout(Duration::from_millis(250)) {
            Ok(Ok(event)) => {
                debug!(?event, "fs event");
                last_event = Some(Instant::now());
                pending = true;
                // Record relevant paths from the event
                for p in event.paths {
                    dirty_paths.push(p);
                }
            }
            Ok(Err(e)) => {
                warn!(error=?e, "watch error");
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                // On timeout, if we had pending events and the quiet period elapsed, sync.
                if pending {
                    if let Some(t) = last_event {
                        if t.elapsed() >= debounce {
                            pending = false;
                            // Drain and dedup changed paths, then selectively sync
                            dirty_paths.sort();
                            dirty_paths.dedup();
                            let mut ok_count = 0usize;
                            let mut err_count = 0usize;
                            for p in dirty_paths.drain(..) {
                                if p.exists() && p.is_file() {
                                    match mirror::sync_template_path(cfg, provider, &p) {
                                        Ok(()) => ok_count += 1,
                                        Err(e) => {
                                            warn!(path=?p, error=?e, "template sync failed");
                                            err_count += 1;
                                        }
                                    }
                                } else {
                                    // Source gone or not a file; remove the mirrored destination if present.
                                    match mirror::remove_dst_for_src(cfg, &p) {
                                        Ok(()) => ok_count += 1,
                                        Err(e) => {
                                            warn!(path=?p, error=?e, "failed to remove mirrored destination");
                                            err_count += 1;
                                        }
                                    }
                                }
                            }
                            // We reached a consistent state; mark healthy (idempotent)
                            if let Err(e) = health::mark_ready(&cfg.status_file) {
                                warn!(error=?e, "failed to update status file after resync");
                            }
                            info!(ok=?ok_count, errors=?err_count, "resync complete after changes");
                        }
                    }
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
