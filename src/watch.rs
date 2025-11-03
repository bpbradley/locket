//! Filesystem watch: monitor templates dir and re-apply sync on changes

use crate::{config::Config, health, provider::SecretsProvider, secrets::Secrets};
use notify::{
    Event, RecursiveMode, Result as NotifyResult, Watcher,
    event::{EventKind, ModifyKind},
    recommended_watcher,
};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};
use tracing::{debug, info, warn};

#[derive(Debug, Clone)]
pub enum Action {
    Inject { src: PathBuf, dst: PathBuf },
    Remove { dst: PathBuf },
    None,
}

pub fn run_watch(
    cfg: &Config,
    secrets: &mut Secrets,
    provider: &dyn SecretsProvider,
) -> anyhow::Result<()> {
    let tpl_dir = Path::new(&cfg.templates_dir);
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
    let mut dirty_paths: Vec<PathBuf> = Vec::new();

    loop {
        match rx.recv_timeout(Duration::from_millis(250)) {
            Ok(Ok(event)) => {
                debug!(?event, "fs event");
                if !file_changed_event(&event.kind) {
                    // Ignore useless events like, open, close, etc.
                    continue;
                }
                last_event = Some(Instant::now());
                pending = true;
                for p in event.paths {
                    dirty_paths.push(p);
                }
            }
            Ok(Err(e)) => warn!(error=?e, "watch error"),
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if pending
                    && let Some(t) = last_event
                    && t.elapsed() >= debounce
                {
                    pending = false;
                    dirty_paths.sort();
                    dirty_paths.dedup();
                    let actions: Vec<Action> = dirty_paths
                        .drain(..)
                        .map(|p| {
                            classify_path_action(secrets, &cfg.templates_dir, &cfg.output_dir, p)
                        })
                        .collect();
                    let mut ok = 0usize;
                    let mut err = 0usize;
                    for a in actions {
                        match a {
                            Action::Inject { src, .. } => {
                                match secrets.inject_file(cfg, provider, &src) {
                                    Ok(true) => ok += 1,
                                    Ok(false) => {
                                        debug!(?src, "inject skipped; src not tracked");
                                    }
                                    Err(e) => {
                                        warn!(error=?e, src=?src, "inject error");
                                        err += 1;
                                    }
                                }
                            }
                            Action::Remove { dst } => match remove_one(&dst) {
                                Ok(()) => ok += 1,
                                Err(e) => {
                                    warn!(error=?e, dst=?dst, "remove error");
                                    err += 1;
                                }
                            },
                            Action::None => {}
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
fn file_changed_event(kind: &EventKind) -> bool {
    matches!(
        kind,
        EventKind::Create(_)
            | EventKind::Remove(_)
            | EventKind::Modify(ModifyKind::Data(_))
            | EventKind::Modify(ModifyKind::Name(_))
            | EventKind::Modify(ModifyKind::Any)
    )
}

fn classify_path_action(
    secrets: &mut Secrets,
    templates_root: &Path,
    output_root: &Path,
    path: PathBuf,
) -> Action {
    if path.exists() && path.is_file() {
        upsert_file(secrets, templates_root, output_root, path)
    } else {
        remove_file(secrets, templates_root, path)
    }
}

/// Insert or update a file mapping when path is under templates_root; returns Inject action or None.
pub fn upsert_file(
    secrets: &mut Secrets,
    templates_root: &Path,
    output_root: &Path,
    src: PathBuf,
) -> Action {
    match src.strip_prefix(templates_root) {
        Ok(rel) if src.is_file() => {
            let dst = output_root.join(rel);
            secrets.files.insert(src.clone(), dst.clone());
            Action::Inject { src, dst }
        }
        _ => Action::None,
    }
}

/// Remove mapping (and destination file) if exists; returns Remove or None.
pub fn remove_file(secrets: &mut Secrets, templates_root: &Path, old_src: PathBuf) -> Action {
    if old_src.strip_prefix(templates_root).is_ok() {
        if let Some(dst) = secrets.files.swap_remove(&old_src) {
            Action::Remove { dst }
        } else {
            Action::None
        }
    } else {
        Action::None
    }
}

fn remove_one(dst: &Path) -> anyhow::Result<()> {
    if dst.exists() && dst.is_file() {
        std::fs::remove_file(dst)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use assert_fs::TempDir;

    #[test]
    fn upsert_inserts_and_returns_action() {
        let tmp = TempDir::new().unwrap();
        let tpl_root = tmp.join("tpl");
        std::fs::create_dir_all(&tpl_root).unwrap();
        let out_root = tmp.join("out");
        let file = tpl_root.join("a.txt");
        std::fs::write(&file, "hi").unwrap();
        let mut secrets = Secrets::new();
        match upsert_file(&mut secrets, &tpl_root, &out_root, file.clone()) {
            Action::Inject { src, dst } => {
                assert_eq!(src, file);
                assert_eq!(dst, out_root.join("a.txt"));
            }
            _ => panic!("expected Inject action"),
        }
        assert!(secrets.files.contains_key(&file));
    }

    #[test]
    fn remove_file_removes_and_returns_action() {
        let tmp = TempDir::new().unwrap();
        let tpl_root = tmp.join("tpl");
        std::fs::create_dir_all(&tpl_root).unwrap();
        let out_root = tmp.join("out");
        let file = tpl_root.join("b.txt");
        std::fs::write(&file, "hi").unwrap();
        let mut secrets = Secrets::new();
        upsert_file(&mut secrets, &tpl_root, &out_root, file.clone());
        match remove_file(&mut secrets, &tpl_root, file.clone()) {
            Action::Remove { dst } => {
                assert_eq!(dst, out_root.join("b.txt"));
            }
            _ => panic!("expected Remove action"),
        }
        assert!(!secrets.files.contains_key(&file));
    }
}
