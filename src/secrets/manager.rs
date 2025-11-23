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

impl PathMapping {
    pub fn new(src: impl AsRef<Path>, dst: impl AsRef<Path>) -> Self {
        Self {
            src: src.as_ref().components().collect(),
            dst: dst.as_ref().components().collect(),
        }
    }
}

impl Default for PathMapping {
    fn default() -> Self {
        Self::new("/templates", "/run/secrets")
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

impl SecretsOpts {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn with_mapping(mut self, mapping: Vec<PathMapping>) -> Self {
        self.mapping = mapping;
        self
    }
    pub fn with_value_dir(mut self, dir: impl AsRef<Path>) -> Self {
        self.value_dir = dir.as_ref().components().collect();
        self
    }
    pub fn with_policy(mut self, policy: InjectFailurePolicy) -> Self {
        self.policy = policy;
        self
    }
    pub fn with_env_value_prefix(mut self, prefix: impl AsRef<str>) -> Self {
        self.env_value_prefix = prefix.as_ref().to_string();
        self
    }
    pub fn validate(&self) -> Result<(), SecretError> {
        let mut sources = Vec::new();
        let mut destinations = Vec::new();

        for m in &self.mapping {
            if m.src
                .components()
                .any(|c| matches!(c, std::path::Component::ParentDir))
            {
                return Err(SecretError::Forbidden(m.src.clone()));
            }
            // Enforce that all source paths exist at startup to avoid ambiguity on what this source is
            if !m.src.exists() {
                return Err(SecretError::SourceMissing(m.src.clone()));
            }
            sources.push(&m.src);
            destinations.push(&m.dst);
        }
        destinations.push(&self.value_dir);

        // Check for feedback loops and self-destruct scenarios
        for src in &sources {
            for dst in &destinations {
                if dst.starts_with(src) {
                    return Err(SecretError::Loop {
                        src: src.to_path_buf(),
                        dst: dst.to_path_buf(),
                    });
                }
                if src.starts_with(dst) {
                    return Err(SecretError::Destructive {
                        src: src.to_path_buf(),
                        dst: dst.to_path_buf(),
                    });
                }
            }
        }

        Ok(())
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

    Ok(PathMapping::new(src, dst))
}

impl SecretsOpts {
    pub fn build(&self) -> Result<Secrets, SecretError> {
        self.validate()?;
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

    pub fn extend_values(&mut self, values: HashMap<String, impl AsRef<str>>) -> &mut Self {
        for (label, template) in values {
            let v = value_source(&self.opts.value_dir, &label, template);
            self.values.insert(v.label.clone(), v);
        }
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

    pub fn collisions(&self) -> Result<(), SecretError> {
        // Collect all secret destinations and label their sources
        // to report in error messages.
        let mut entries: Vec<(&Path, String)> = Vec::new();

        for file in self.fs.iter_files() {
            entries.push((&file.dst, format!("File({:?})", file.src)));
        }

        for val in self.values.values() {
            entries.push((&val.dst, format!("Value({})", val.label)));
        }

        // Sort Lexicographically. This groups collisions and parent/child conflicts together.
        entries.sort_by_key(|(path, _)| *path);

        // Linear scan
        for i in 0..entries.len().saturating_sub(1) {
            let (curr_path, curr_src) = &entries[i];
            let (next_path, next_src) = &entries[i + 1];

            // Two secrets share a destination
            if curr_path == next_path {
                return Err(SecretError::Collision {
                    first: curr_src.clone(),
                    second: next_src.clone(),
                    dst: curr_path.to_path_buf(),
                });
            }

            // Finds structural conflicts where one secret maps to a path
            // that is a parent directory of another secret's path.
            // i.e. /secrets/foo and /secrets/foo/bar.txt
            if next_path.starts_with(curr_path) {
                return Err(SecretError::StructureConflict {
                    blocker: curr_src.clone(),
                    blocker_path: curr_path.to_path_buf(),
                    blocked: next_src.clone(),
                    blocked_path: next_path.to_path_buf(),
                });
            }
        }

        Ok(())
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
    /// TODO: There are some edges with how we bubble delete here.
    /// For example, since we traverse bottom up, if there are empty
    /// sibling directories, we wont remove_dir won't remove them
    /// and we will exit with DirectoryNotEmpty. We could do a more thorough
    /// traversal to catch these, but overkill for an edge.
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

        // Fallback to remove + write
        debug!(?old, ?new, "fallback move via remove + write");
        self.on_remove(old)?;
        self.on_write(provider, new)?;

        Ok(())
    }

    fn on_write(&mut self, provider: &dyn SecretsProvider, src: &Path) -> Result<(), SecretError> {
        if src.is_dir() {
            debug!(?src, "directory write event; scanning for children");
            for entry in walkdir::WalkDir::new(src)
                .into_iter()
                .filter_map(|e| e.ok())
                .filter(|e| e.file_type().is_file())
            {
                // Recursion.. Treat every child file as its own Write event.
                // Should only ever be one level deep here, since we are already
                // inside a directory write event.
                self.on_write(provider, entry.path())?;
            }
            return Ok(());
        }
        // Tiny race condition here, if file is removed while we are processing it..
        // Only a possible issue with inject failure policy of Error.
        // Otherwise, this will lead to eventual consistency on the next processing event
        if src.exists()
            && let Some(file) = self.fs.upsert(src)
        {
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
            FsEvent::Write(src) => self.on_write(provider, &normalize(src)),
            FsEvent::Remove(src) => self.on_remove(&normalize(src)),
            FsEvent::Move { from, to } => self.on_move(provider, &normalize(from), &normalize(to)),
        }
    }
}

fn normalize(path: impl AsRef<Path>) -> PathBuf {
    path.as_ref().components().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn validate_fails_source_missing() {
        let tmp = tempdir().unwrap();
        let missing_src = tmp.path().join("ghost");
        let dst = tmp.path().join("out");

        let opts = SecretsOpts::default().with_mapping(vec![PathMapping::new(&missing_src, &dst)]);
        assert!(matches!(
            opts.validate(),
            Err(SecretError::SourceMissing(p)) if p == missing_src
        ));
    }

    #[test]
    fn validate_fails_forbidden_relative_path() {
        let tmp = tempdir().unwrap();
        let src = tmp.path().join("templates");
        std::fs::create_dir_all(&src).unwrap();
        let bad_src = src.join("..").join("passwd");

        let opts = SecretsOpts::default().with_mapping(vec![PathMapping::new(&bad_src, "out")]);
        assert!(matches!(
            opts.validate(),
            Err(SecretError::Forbidden(p)) if p == bad_src
        ));
    }

    #[test]
    fn validate_fails_loop_dst_inside_src() {
        let tmp = tempdir().unwrap();
        let src = tmp.path().join("templates");
        let dst = src.join("nested_out");

        std::fs::create_dir_all(&src).unwrap();

        let opts = SecretsOpts::default().with_mapping(vec![PathMapping::new(&src, &dst)]);

        assert!(matches!(
            opts.validate(),
            Err(SecretError::Loop { src: s, dst: d }) if s == src && d == dst
        ));
    }

    #[test]
    fn validate_fails_destructive() {
        let tmp = tempdir().unwrap();
        let dst = tmp.path().join("out");
        let src = dst.join("templates");

        std::fs::create_dir_all(&src).unwrap();

        let opts = SecretsOpts::default().with_mapping(vec![PathMapping::new(&src, &dst)]);

        assert!(matches!(
            opts.validate(),
            Err(SecretError::Destructive { src: s, dst: d }) if s == src && d == dst
        ));
    }

    #[test]
    fn validate_fails_value_dir_loop() {
        let tmp = tempdir().unwrap();
        let src = tmp.path().join("templates");
        std::fs::create_dir_all(&src).unwrap();

        let dst = tmp.path().join("safe_out");
        let bad_value_dir = src.join("values");

        let opts = SecretsOpts::default()
            .with_mapping(vec![PathMapping::new(&src, &dst)])
            .with_value_dir(bad_value_dir.clone());

        assert!(matches!(
            opts.validate(),
            Err(SecretError::Loop { src: s, dst: d }) if s == src && d == bad_value_dir
        ));
    }

    #[test]
    fn validate_succeeds_valid_config() {
        let tmp = tempdir().unwrap();
        let src = tmp.path().join("templates");
        let dst = tmp.path().join("out");

        std::fs::create_dir_all(&src).unwrap();

        let opts = SecretsOpts::default().with_mapping(vec![PathMapping::new(src, dst)]);

        assert!(opts.validate().is_ok());
    }
}
