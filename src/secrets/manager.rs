use crate::provider::SecretsProvider;
use crate::secrets::fs::SecretFs;
use crate::secrets::types::{
    InjectFailurePolicy, Injectable, SecretError, SecretValue, collect_value_sources_from_env,
    value_source,
};
use clap::Args;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tracing::{debug, warn};

#[derive(Debug, Clone)]
pub struct PathMapping {
    pub src: PathBuf,
    pub dst: PathBuf,
}

impl Default for PathMapping {
    fn default() -> Self {
        Self {
            src: PathBuf::from("/templates"),
            dst: PathBuf::from("/run/secrets"),
        }
    }
}

/// CLI / config options.
#[derive(Debug, Clone, Args)]
pub struct SecretsOpts {
    #[arg(
        long = "map", 
        value_parser = parse_mapping,
        env = "SECRET_MAP", 
        value_delimiter = ',',
        default_value = "/templates:/run/secrets"
    )]
    pub mapping: Vec<PathMapping>,
    #[arg(long = "out", env = "VALUE_OUTPUT_DIR", default_value = "/run/secrets")]
    pub value_dir: PathBuf,
    #[arg(long, env = "VALUE_PREFIX", default_value = "secret_")]
    pub env_value_prefix: String,
    #[arg(
        long = "inject-policy",
        env = "INJECT_POLICY",
        value_enum,
        default_value_t = InjectFailurePolicy::CopyUnmodified
    )]
    pub policy: InjectFailurePolicy,
}

impl Default for SecretsOpts {
    fn default() -> Self {
        Self {
            mapping: vec![PathMapping::default()],
            value_dir: PathBuf::from("/run/secrets"),
            env_value_prefix: "secret_".into(),
            policy: InjectFailurePolicy::CopyUnmodified,
        }
    }
}

/// Parse a path mapping from a string of the form "SRC:DST" or "SRC=DST".
fn parse_mapping(s: &str) -> Result<PathMapping, String> {
    let (src, dst) = s
        .split_once(':')
        .or_else(|| s.split_once('=')) // Allow '=' if there is no ':' or multiple (Windows)
        .ok_or_else(|| {
            format!(
                "Invalid mapping format '{}'. Expected SRC:DST or SRC=DST",
                s
            )
        })?;

    Ok(PathMapping {
        src: PathBuf::from(src),
        dst: PathBuf::from(dst),
    })
}

impl SecretsOpts {
    pub fn build(&self) -> Result<Secrets, SecretError> {
        Ok(Secrets::new(self.clone()))
    }
}

/// Filesystem events for SecretFs
#[derive(Debug, Ord, PartialOrd, Eq, PartialEq)]
pub enum FsEvent {
    Write(PathBuf),
    Remove(PathBuf),
    Move { from: PathBuf, to: PathBuf },
}

pub struct Secrets {
    opts: SecretsOpts,
    fs: SecretFs,
    values: HashMap<String, SecretValue>,
}

impl Secrets {
    pub fn new(opts: SecretsOpts) -> Self {
        let fs = SecretFs::new(opts.mapping.clone());
        let mut secrets = Self {
            opts,
            fs,
            values: HashMap::new(),
        };
        let envs =
            collect_value_sources_from_env(&secrets.opts.value_dir, &secrets.opts.env_value_prefix);
        for v in envs {
            secrets.values.insert(v.label.clone(), v);
        }
        secrets
    }

    pub fn options(&self) -> &SecretsOpts {
        &self.opts
    }

    pub fn add_value(&mut self, label: &str, template: impl AsRef<str>) -> &mut Self {
        let v = value_source(&self.opts.value_dir, label, template);
        self.values.insert(v.label.clone(), v);
        self
    }

    /// Inject all known secrets (values + files).
    pub fn inject_all(&self, provider: &dyn SecretsProvider) -> Result<(), SecretError> {
        let policy = self.opts.policy;

        // value-backed secrets
        for v in self.values.values() {
            v.inject(policy, provider)?;
        }

        // file-backed secrets
        for f in self.fs.iter_files() {
            f.inject(policy, provider)?;
        }

        Ok(())
    }

    /// Find collisions on destination paths.
    pub fn collisions(&self) -> Vec<PathBuf> {
        use std::collections::HashMap;
        let mut counts: HashMap<PathBuf, usize> = HashMap::new();

        // values
        for v in self.values.values() {
            *counts.entry(v.dst.clone()).or_insert(0) += 1;
        }

        // files
        for f in self.fs.iter_files() {
            *counts.entry(f.dst.clone()).or_insert(0) += 1;
        }

        counts
            .into_iter()
            .filter_map(|(p, n)| (n > 1).then_some(p))
            .collect()
    }

    fn on_remove(&mut self, src: &Path) -> Result<(), SecretError> {
        let removed = self.fs.remove(src);
        if removed.is_empty() {
            debug!(
                ?src,
                "event: path removed but no secrets were tracked there"
            );
            return Ok(());
        }

        for file in &removed {
            file.remove()?;
            debug!(?file.dst, "event: removed secret file");
        }

        // Attempt to bubble delete empty parent dirs up to the event implied ceiling.
        if let Some(ceiling) = self.fs.resolve(src) {
            let mut candidates = std::collections::HashSet::new();
            for file in &removed {
                if let Some(parent) = file.dst.parent() {
                    candidates.insert(parent.to_path_buf());
                }
            }

            for dir in candidates {
                if dir.starts_with(&ceiling) && dir.exists() {
                    self.bubble_delete(dir, &ceiling);
                }
            }
        }
        Ok(())
    }

    fn bubble_delete(&self, start_dir: PathBuf, ceiling: &Path) {
        let mut current = start_dir;

        loop {
            if !current.starts_with(ceiling) {
                break;
            }
            match std::fs::remove_dir(&current) {
                Ok(_) => {
                    if current == ceiling {
                        break;
                    }
                    if let Some(parent) = current.parent() {
                        current = parent.to_path_buf();
                    } else {
                        break;
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::DirectoryNotEmpty => {
                    break;
                }
                Err(_) => {
                    break;
                }
            }
        }
    }

    fn on_move(
        &mut self,
        provider: &dyn SecretsProvider,
        old: &Path,
        new: &Path,
    ) -> Result<(), SecretError> {
        if let Some((from, to)) = self.fs.try_rebase(old, new) {
            debug!(?from, ?to, "attempting rename");

            if let Some(p) = to.parent() {
                std::fs::create_dir_all(p)?;
            }

            match std::fs::rename(&from, &to) {
                Ok(_) => {
                    debug!(?old, ?new, "moved");
                    if let Some(parent) = from.parent() {
                        // We calculate the ceiling based on the OLD source path
                        if let Some(ceiling) = self.fs.resolve(old) {
                            // If the old file/dir was inside the ceiling, we bubble up
                            // We start vacuuming at 'parent', stopping at 'ceiling's parent
                            if let Some(ceil_parent) = ceiling.parent()
                                && parent.starts_with(ceil_parent)
                            {
                                self.bubble_delete(parent.to_path_buf(), ceil_parent);
                            }
                        }
                    }
                    return Ok(());
                }
                Err(e) => {
                    warn!(error=?e, "move failed; falling back to reinjection");
                    // Rollback memory state so we can start fresh
                    self.fs.remove(new);
                }
            }
        }

        self.on_remove(old)?;

        // new.is_dir() here is a small race condition. If new is removed before we can process it, we will
        // treat it as a file and fail to inject anything.
        // However, this will still lead to eventual consistency, as the next write event
        // should show that the file was removed. In order to fix this, I could
        // add context to the FsEvent to indicate that the file is a directory at event-time.
        // This will still require graceful handling, but at least it would be correct.
        if new.is_dir() {
            debug!(?new, "scanning new directory location");
            for entry in walkdir::WalkDir::new(new)
                .into_iter()
                .filter_map(|e| e.ok())
                .filter(|e| e.file_type().is_file())
            {
                self.on_write(provider, entry.path())?;
            }
        } else {
            self.on_write(provider, new)?;
        }

        Ok(())
    }

    fn on_write(&mut self, provider: &dyn SecretsProvider, src: &Path) -> Result<(), SecretError> {
        // Only react to files, not directories.
        if src.is_dir() {
            debug!(?src, "event: skipping directory write");
            return Ok(());
        }
        if let Some(file) = self.fs.upsert(src) {
            file.inject(self.opts.policy, provider)?;
        }
        Ok(())
    }

    pub fn handle_fs_event(
        &mut self,
        provider: &dyn SecretsProvider,
        ev: FsEvent,
    ) -> Result<(), SecretError> {
        match ev {
            FsEvent::Write(src) => self.on_write(provider, &src),
            FsEvent::Remove(src) => self.on_remove(&src),
            FsEvent::Move { from, to } => self.on_move(provider, &from, &to),
        }
    }
}
