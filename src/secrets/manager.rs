//! Secret file management.
//!
//! This module defines the `SecretFileManager`, which is responsible for
//! managing secret files based on file-backed templates containing secret references.

use crate::events::{EventHandler, FsEvent, HandlerError};
use crate::path::{AbsolutePath, CanonicalPath, PathMapping};
use crate::provider::SecretsProvider;
use crate::secrets::registry::SecretFileRegistry;
use crate::secrets::{MemSize, Secret, SecretError, SecretSource, file::SecretFile};
use crate::template::Template;
use crate::write::FileWriter;
use async_trait::async_trait;
use clap::{Args, ValueEnum};
use secrecy::ExposeSecret;
use std::borrow::Cow;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tracing::{debug, info, warn};

#[derive(Debug, Clone, Args)]
pub struct SecretFileOpts {
    /// Mapping of source paths (holding secret templates)
    /// to destination paths (where secrets are materialized and reflected)
    /// in the form `SRC:DST` or `SRC=DST`. Multiple mappings can be
    /// provided, separated by commas, or supplied multiple times as arguments.
    /// e.g. `--map /templates:/run/secrets/locket/app --map /other_templates:/run/secrets/locket/other`
    #[arg(
        long = "map",
        env = "SECRET_MAP",
        value_delimiter = ',',
        default_value = "/templates:/run/secrets/locket",
        hide_env_values = true
    )]
    pub mapping: Vec<PathMapping>,

    /// Additional secret values specified as LABEL=SECRET_TEMPLATE
    /// Multiple values can be provided, separated by commas.
    /// Or supplied multiple times as arguments.
    /// Loading from file is supported via `LABEL=@/path/to/file`.
    /// e.g. `--secret db_password={{op://..}} --secret api_key={{op://..}}`
    #[arg(
        long = "secret",
        env = "LOCKET_SECRETS",
        value_name = "label={{template}}",
        value_delimiter = ',',
        hide_env_values = true
    )]
    pub secrets: Vec<Secret>,

    /// Directory where secret values (literals) are materialized
    #[arg(
        long = "out",
        env = "DEFAULT_SECRET_DIR",
        default_value = "/run/secrets/locket"
    )]
    pub secret_dir: AbsolutePath,
    #[arg(
        long = "inject-policy",
        env = "INJECT_POLICY",
        value_enum,
        default_value_t = InjectFailurePolicy::CopyUnmodified
    )]
    /// Policy for handling injection failures
    pub policy: InjectFailurePolicy,
    /// Maximum allowable size for a template file. Files larger than this will be rejected.
    /// Supports human-friendly suffixes like K, M, G (e.g. 10M = 10 Megabytes).
    #[arg(long = "max-file-size", env = "MAX_FILE_SIZE", default_value = "10M")]
    pub max_file_size: MemSize,

    /// File writing permissions
    #[command(flatten)]
    pub writer: FileWriter,
}

#[derive(Copy, Clone, Debug, ValueEnum, Default)]
pub enum InjectFailurePolicy {
    /// Injection failures are treated as errors and will abort the process
    Error,
    /// On injection failure, copy the unmodified secret to destination
    #[default]
    CopyUnmodified,
    /// On injection failure, just log a warning and proceed with the secret ignored
    Ignore,
}

impl SecretFileOpts {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn with_mapping(mut self, mapping: Vec<PathMapping>) -> Self {
        self.mapping = mapping;
        self
    }
    pub fn with_secret_dir(mut self, dir: AbsolutePath) -> Self {
        self.secret_dir = dir;
        self
    }
    pub fn with_policy(mut self, policy: InjectFailurePolicy) -> Self {
        self.policy = policy;
        self
    }
    pub fn with_secrets(mut self, secrets: Vec<Secret>) -> Self {
        self.secrets = secrets;
        self
    }
    pub fn with_writer(mut self, writer: FileWriter) -> Self {
        self.writer = writer;
        self
    }
    pub fn resolve(&mut self) -> Result<(), SecretError> {
        let mut sources = Vec::new();
        let mut destinations = Vec::new();

        for m in &self.mapping {
            sources.push(m.src());
            destinations.push(m.dst());
        }
        destinations.push(&self.secret_dir);

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

impl Default for SecretFileOpts {
    fn default() -> Self {
        Self {
            mapping: Vec::new(),
            secret_dir: AbsolutePath::new("./secrets"),
            secrets: Vec::new(),
            policy: InjectFailurePolicy::CopyUnmodified,
            max_file_size: MemSize::default(),
            writer: FileWriter::default(),
        }
    }
}

/// Manager for secret files, responsible for resolving and materializing secrets
/// based on templates and secret references.
///
/// It maintains a registry of secret files, handles file system events,
/// and interacts with a secrets provider to fetch secret values.
pub struct SecretFileManager {
    opts: SecretFileOpts,
    registry: SecretFileRegistry,
    literals: Vec<SecretFile>,
    provider: Arc<dyn SecretsProvider>,
}

impl SecretFileManager {
    pub fn new(
        opts: SecretFileOpts,
        provider: Arc<dyn SecretsProvider>,
    ) -> Result<Self, SecretError> {
        let mut pinned = Vec::new();
        let mut literals = Vec::new();

        for s in &opts.secrets {
            let f = SecretFile::from_secret(s.clone(), &opts.secret_dir, opts.max_file_size)?;
            match f.source() {
                SecretSource::File(_) => pinned.push(f),
                SecretSource::Literal { .. } => literals.push(f),
            }
        }

        let registry = SecretFileRegistry::new(opts.mapping.clone(), pinned, opts.max_file_size);

        let manager = Self {
            opts,
            registry,
            literals,
            provider,
        };

        Ok(manager)
    }

    pub fn iter_secrets(&self) -> impl Iterator<Item = &SecretFile> {
        self.registry.iter().chain(self.literals.iter())
    }

    pub fn options(&self) -> &SecretFileOpts {
        &self.opts
    }

    pub fn sources(&self) -> Vec<PathBuf> {
        let pinned = self
            .registry
            .iter()
            .filter_map(|f| f.source().path().map(|p| p.to_path_buf()));

        let mapped = self.opts.mapping.iter().map(|m| m.src().to_path_buf());

        pinned.chain(mapped).collect()
    }

    pub fn add_value(&mut self, label: &str, template: impl AsRef<str>) -> &mut Self {
        let v = SecretFile::from_template(
            label.to_string(),
            template.as_ref().to_string(),
            &self.opts.secret_dir,
        );
        self.literals.push(v);
        self
    }

    pub async fn resolve(&self, file: &SecretFile) -> Result<String, SecretError> {
        let f = file.clone();
        let content =
            tokio::task::spawn_blocking(move || f.content().map(|c| c.into_owned())).await??;

        let tpl = Template::new(&content);
        let keys = tpl.keys();
        let has_keys = !keys.is_empty();

        let candidates: Vec<&str> = if has_keys {
            keys.into_iter().collect()
        } else {
            vec![content.trim()]
        };

        let references: Vec<&str> = candidates
            .into_iter()
            .filter(|k| self.provider.accepts_key(k))
            .collect();

        if references.is_empty() {
            debug!(dst=?file.dest(), "no resolveable secrets found; passing through");
            return Ok(content);
        }

        info!(dst=?file.dest(), count=references.len(), "fetching secrets");
        let secrets_map = self.provider.fetch_map(&references).await?;

        let output = if has_keys {
            tpl.render_with(|k| secrets_map.get(k).map(|s| s.expose_secret()))
        } else {
            match secrets_map.get(content.trim()) {
                Some(val) => Cow::Borrowed(val.expose_secret()),
                None => {
                    warn!(dst=?file.dest(), "provider returned success but secret value was missing");
                    Cow::Borrowed(content.as_str())
                }
            }
        };

        Ok(output.into_owned())
    }

    pub async fn materialize(&self, file: &SecretFile, content: String) -> Result<(), SecretError> {
        let writer = self.opts.writer.clone();
        let dest = file.dest().clone();
        let bytes = content.into_bytes();

        tokio::task::spawn_blocking(move || writer.atomic_write(&dest, &bytes)).await??;

        Ok(())
    }

    async fn handle_policy(
        &self,
        file: &SecretFile,
        err: SecretError,
        policy: InjectFailurePolicy,
    ) -> Result<(), SecretError> {
        if let SecretError::SourceMissing(path) = &err {
            debug!("Source doesn't exist: {:?}. Ignoring.", path);
            return Ok(());
        }

        match policy {
            InjectFailurePolicy::Error => Err(err),
            InjectFailurePolicy::CopyUnmodified => {
                warn!(
                    src = ?file.source().label(),
                    dst = ?file.dest(),
                    error = ?err,
                    "injection failed; policy=copy-unmodified. Reverting to raw copy."
                );
                let f = file.clone();
                let raw = tokio::task::spawn_blocking(move || {
                    // unwrap_or_default to take IO errors during fallback
                    // if we can't read the source, we just don't write anything
                    f.content().map(|c| c.into_owned()).unwrap_or_default()
                })
                .await?;

                if !raw.is_empty() {
                    self.materialize(file, raw).await?;
                }
                Ok(())
            }
            InjectFailurePolicy::Ignore => {
                warn!(src = ?file.source().label(), dst = ?file.dest(), error = ?err, "injection failed; ignoring");
                Ok(())
            }
        }
    }

    pub async fn process(&self, file: &SecretFile) -> Result<(), SecretError> {
        match self.resolve(file).await {
            Ok(content) => {
                if let Err(e) = self.materialize(file, content).await {
                    return self.handle_policy(file, e, self.opts.policy).await;
                }
                Ok(())
            }
            Err(e) => self.handle_policy(file, e, self.opts.policy).await,
        }
    }

    pub async fn inject_all(&self) -> Result<(), SecretError> {
        for file in self.iter_secrets() {
            self.process(file).await?;
        }
        Ok(())
    }

    pub fn collisions(&self) -> Result<(), SecretError> {
        // Collect all secret destinations and label their sources
        let mut entries: Vec<(&Path, String)> = Vec::new();

        for file in self.iter_secrets() {
            entries.push((file.dest(), format!("File({:?})", file.source().label())));
        }

        // Sort Lexicographically
        entries.sort_by_key(|(path, _)| *path);

        // Linear scan
        for i in 0..entries.len().saturating_sub(1) {
            let (curr_path, curr_src) = &entries[i];
            let (next_path, next_src) = &entries[i + 1];

            // Collision
            if curr_path == next_path {
                return Err(SecretError::Collision {
                    first: curr_src.clone(),
                    second: next_src.clone(),
                    dst: curr_path.to_path_buf(),
                });
            }

            // Nesting conflict
            if next_path.starts_with(curr_path) {
                return Err(SecretError::StructureConflict {
                    blocker: curr_src.clone(),
                    blocked: next_src.clone(),
                });
            }
        }

        Ok(())
    }

    fn handle_remove(&mut self, src: AbsolutePath) -> Result<(), SecretError> {
        let removed = self.registry.remove(&src);
        if removed.is_empty() {
            debug!(
                ?src,
                "event: path removed but no secrets were tracked there"
            );
            return Ok(());
        }

        for file in &removed {
            let dst = file.dest();
            if dst.exists() {
                std::fs::remove_file(dst)?;
            }
            debug!("event: removed secret file: {:?}", file.dest());
        }

        // Clean up empty parent directories
        if let Some(ceiling) = self.registry.resolve(&src) {
            self.cleanup_parents(removed, &ceiling);
        }

        Ok(())
    }

    fn cleanup_parents(&self, removed_files: Vec<SecretFile>, ceiling: &Path) {
        let mut candidates = std::collections::HashSet::new();
        for file in removed_files {
            if let Some(parent) = file.dest().parent() {
                candidates.insert(parent.to_path_buf());
            }
        }

        for dir in candidates {
            if dir.starts_with(ceiling) && dir.exists() {
                self.bubble_delete(dir, ceiling);
            }
        }
    }

    fn bubble_delete(&self, start_dir: PathBuf, ceiling: &Path) {
        let mut current = start_dir;
        loop {
            if !current.starts_with(ceiling) {
                break;
            }
            // Attempt removal; stop if not empty or other error
            match std::fs::remove_dir(&current) {
                Ok(_) => {
                    if current == ceiling {
                        break;
                    }
                    match current.parent() {
                        Some(p) => current = p.to_path_buf(),
                        None => break,
                    }
                }
                Err(_) => break,
            }
        }
    }

    async fn handle_move(
        &mut self,
        old: AbsolutePath,
        new: CanonicalPath,
    ) -> Result<(), SecretError> {
        if let Some((from_dst, to_dst)) = self.registry.try_rebase(&old, &new) {
            debug!(?from_dst, ?to_dst, "attempting optimistic rename");

            if let Some(p) = to_dst.parent() {
                tokio::fs::create_dir_all(p).await?;
            }

            match tokio::fs::rename(&from_dst, &to_dst).await {
                Ok(_) => {
                    debug!(?old, ?new, "moved");
                    // Cleanup old parent dirs
                    if let Some(parent) = from_dst.parent()
                        && let Some(ceiling) = self.registry.resolve(&old)
                        && let Some(ceil_parent) = ceiling.parent()
                    {
                        self.bubble_delete(parent.to_path_buf(), ceil_parent);
                    }
                    return Ok(());
                }
                Err(e) => {
                    warn!(error=?e, "move failed; falling back to reinjection");
                    // rollback registry state
                    self.registry.remove(&new);
                    if from_dst.exists() {
                        let _ = tokio::fs::remove_file(&from_dst).await;
                    }
                }
            }
        }

        // Fallback
        debug!(?old, ?new, "fallback move via remove + write");
        self.handle_remove(old)?;
        self.handle_write(new).await?;

        Ok(())
    }

    async fn handle_write(&mut self, src: CanonicalPath) -> Result<(), SecretError> {
        if src.is_dir() {
            debug!(?src, "directory write event; scanning for children");
            let entries: Vec<PathBuf> = walkdir::WalkDir::new(&src)
                .into_iter()
                .filter_map(|e| e.ok())
                .filter(|e| e.file_type().is_file())
                .map(|e| e.path().to_path_buf())
                .collect();

            for entry in entries {
                if let Ok(canon_entry) = CanonicalPath::try_new(entry) {
                    // Pinning is required for async recursion
                    Box::pin(self.handle_write(canon_entry)).await?;
                }
            }
            return Ok(());
        }

        match self.registry.upsert(&src)? {
            Some(file) => {
                self.process(&file).await?;
            }
            None => {
                // File ignored
            }
        }
        Ok(())
    }
}

/// File system watch handler for SecretFileManager.
///
/// It responds to file system events by updating or removing
/// the corresponding secret files as needed. Its purpose is to
/// reflect changes in template source files to the managed secret files.
#[async_trait]
impl EventHandler for SecretFileManager {
    fn paths(&self) -> Vec<PathBuf> {
        self.sources()
    }

    async fn handle(&mut self, events: Vec<FsEvent>) -> Result<(), HandlerError> {
        for event in events {
            let result = match event {
                FsEvent::Write(src) => match CanonicalPath::try_new(&src) {
                    Ok(canon) => self.handle_write(canon).await,
                    Err(e) => {
                        debug!(?src, "write/create event for missing file; ignoring: {}", e);
                        Ok(())
                    }
                },
                FsEvent::Remove(src) => {
                    let abs = AbsolutePath::new(src);
                    self.handle_remove(abs)
                }

                FsEvent::Move { from, to } => {
                    let old_abs = AbsolutePath::new(from);
                    match CanonicalPath::try_new(&to) {
                        Ok(new_canon) => self.handle_move(old_abs, new_canon).await,
                        Err(e) => {
                            // Moved to path is missing. Treat as a delete of old file.
                            debug!(
                                ?to,
                                "move destination missing; downgrading to remove: {}", e
                            );
                            self.handle_remove(old_abs)
                        }
                    }
                }
            };

            if let Err(e) = result {
                warn!(error = ?e, "failed to process fs event");
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secrets::MemSize;
    use std::path::Path;
    use std::str::FromStr;

    #[test]
    fn secret_value_sanitization() {
        let root = Path::new("/");

        let v = SecretFile::from_template("Db_Password".to_string(), "".to_string(), root);
        assert_eq!(v.dest(), Path::new("/Db_Password")); // absolute() call in from_template might affect this depending on platform, but logic holds

        let v = SecretFile::from_template("A/B/C".to_string(), "".to_string(), root);
        assert_eq!(v.dest(), Path::new("/ABC"));

        let v = SecretFile::from_template("weird name".to_string(), "".to_string(), root);
        assert_eq!(v.dest(), Path::new("/weird name"));

        let v = SecretFile::from_template("..//--__".to_string(), "".to_string(), root);
        assert_eq!(v.dest(), Path::new("/..--__"));
    }

    #[test]
    fn test_size_parsing() {
        assert_eq!(MemSize::from_str("100").unwrap().bytes, 100);
        assert_eq!(MemSize::from_str("1k").unwrap().bytes, 1024);
        assert_eq!(MemSize::from_str("10M").unwrap().bytes, 10 * 1024 * 1024);
    }
}
