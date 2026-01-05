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
use std::path::PathBuf;
use std::sync::Arc;
use tracing::{debug, info, warn};

#[derive(Debug, Clone, Args)]
pub struct SecretFileOpts {
    /// Mapping of source paths to destination paths.
    ///
    /// Maps sources (holding secret templates) to destination paths
    /// (where secrets are materialized) in the form `SRC:DST` or `SRC=DST`.
    ///
    /// Multiple mappings can be provided, separated by commas, or supplied
    /// multiple times as arguments.
    ///
    /// Example: `--map /templates:/run/secrets/app`
    ///
    /// **CLI Default:** None
    /// **Docker Default:** `/templates:/run/secrets/locket`
    #[arg(
        long = "map",
        env = "SECRET_MAP",
        value_delimiter = ',',
        hide_env_values = true
    )]
    pub mapping: Vec<PathMapping>,

    /// Additional secret values specified as LABEL=SECRET_TEMPLATE
    ///
    /// Multiple values can be provided, separated by commas.
    /// Or supplied multiple times as arguments.
    ///
    /// Loading from file is supported via `LABEL=@/path/to/file`.
    /// Example: `--secret db_password={{op://..}} --secret api_key={{op://..}}`
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
        default_value = SecretFileOpts::default().secret_dir.to_string()
    )]
    pub secret_dir: AbsolutePath,

    /// Policy for handling injection failures
    #[arg(
        long = "inject-policy",
        env = "INJECT_POLICY",
        value_enum,
        default_value_t = InjectFailurePolicy::CopyUnmodified
    )]
    pub policy: InjectFailurePolicy,

    /// Maximum allowable size for a template file. Files larger than this will be rejected.
    ///
    /// Supports human-friendly suffixes like K, M, G (e.g. 10M = 10 Megabytes).
    #[arg(long = "max-file-size", env = "MAX_FILE_SIZE", default_value = MemSize::default().to_string())]
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
    fn resolve(&mut self) -> Result<(), SecretError> {
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
            #[cfg(target_os = "linux")]
            secret_dir: AbsolutePath::new("/run/secrets/locket"),
            #[cfg(target_os = "macos")]
            secret_dir: AbsolutePath::new("/private/tmp/locket"),
            #[cfg(not(any(target_os = "linux", target_os = "macos")))]
            secret_dir: AbsolutePath::new("./secrets"), // Fallback
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
        mut opts: SecretFileOpts,
        provider: Arc<dyn SecretsProvider>,
    ) -> Result<Self, SecretError> {
        let mut pinned = Vec::new();
        let mut literals = Vec::new();

        opts.resolve()?;

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

        manager.collisions()?;

        Ok(manager)
    }

    pub fn iter_secrets(&self) -> impl Iterator<Item = &SecretFile> {
        self.registry.iter().chain(self.literals.iter())
    }

    pub fn options(&self) -> &SecretFileOpts {
        &self.opts
    }

    pub fn sources(&self) -> Vec<AbsolutePath> {
        let pinned = self
            .registry
            .iter()
            .filter_map(|f| f.source().path().map(|p| AbsolutePath::from(p.clone())));
        let mapped = self
            .opts
            .mapping
            .iter()
            .map(|m| AbsolutePath::from(m.src().clone()));

        pinned.chain(mapped).collect()
    }

    async fn resolve(&self, file: &SecretFile) -> Result<String, SecretError> {
        let f = file.clone();
        let content =
            tokio::task::spawn_blocking(move || f.content().map(|c| c.into_owned())).await??;

        let tpl = Template::parse(&content, &*self.provider);

        if tpl.has_secrets() {
            let references_to_fetch = tpl.references();

            info!(dst=?file.dest(), count=references_to_fetch.len(), "fetching secrets from template");
            let secrets_map = self.provider.fetch_map(&references_to_fetch).await?;

            let output = tpl.render_with(|k| secrets_map.get(k).map(|s| s.expose_secret()));
            Ok(output.into_owned())
        } else {
            // Try to parse the entire trimmed content as a single reference.
            if let Some(reference) = self.provider.parse(content.trim()) {
                info!(dst=?file.dest(), "fetching bare secret");

                let secrets_map = self
                    .provider
                    .fetch_map(std::slice::from_ref(&reference))
                    .await?;

                match secrets_map.get(&reference) {
                    Some(val) => Ok(val.expose_secret().to_string()),
                    None => {
                        warn!(dst=?file.dest(), "provider returned success but secret value was missing");
                        Ok(content) // Fallback to original content
                    }
                }
            } else {
                // Not a template and not a bare secret, so just return the original content.
                debug!(dst=?file.dest(), "no resolvable secrets found; passing through");
                Ok(content)
            }
        }
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

    fn collisions(&self) -> Result<(), SecretError> {
        // Collect all secret destinations and label their sources
        let mut entries: Vec<(&AbsolutePath, String)> = Vec::new();

        for file in self.iter_secrets() {
            entries.push((file.dest(), format!("File({})", file.source().label())));
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
        if let Some(ceiling) = self.registry.resolve(src.clone()) {
            self.cleanup_parents(removed, &ceiling);
        }

        Ok(())
    }

    fn cleanup_parents(&self, removed_files: Vec<SecretFile>, ceiling: &AbsolutePath) {
        let mut candidates = std::collections::HashSet::new();
        for file in removed_files {
            if let Some(parent) = file.dest().parent() {
                candidates.insert(parent.to_path_buf());
            }
        }

        for dir in candidates {
            if let Ok(candidate) = CanonicalPath::try_new(dir)
                && candidate.starts_with(ceiling)
            {
                self.bubble_delete(candidate.into(), ceiling);
            }
        }
    }

    fn bubble_delete(&self, start_dir: AbsolutePath, ceiling: &AbsolutePath) {
        let mut current = start_dir;
        loop {
            if !current.starts_with(ceiling) {
                break;
            }
            // Attempt removal; stop if not empty or other error
            match std::fs::remove_dir(&current) {
                Ok(_) => {
                    if current == *ceiling {
                        break;
                    }
                    match current.parent() {
                        Some(p) => current = p,
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
        if let Some((from_dst, to_dst)) = self.registry.try_rebase(&old, &new.clone().into()) {
            debug!(?from_dst, ?to_dst, "attempting optimistic rename");

            if let Some(p) = to_dst.parent() {
                tokio::fs::create_dir_all(p).await?;
            }

            match tokio::fs::rename(&from_dst, &to_dst).await {
                Ok(_) => {
                    debug!(?old, ?new, "moved");
                    // Cleanup old parent dirs
                    if let Some(parent) = from_dst.parent()
                        && let Some(ceiling) = self.registry.resolve(old.clone())
                        && let Some(ceil_parent) = ceiling.parent()
                    {
                        self.bubble_delete(parent, &ceil_parent);
                    }
                    return Ok(());
                }
                Err(e) => {
                    warn!(error=?e, "move failed; falling back to reinjection");
                    // rollback registry state
                    self.registry.remove(&new.clone().into());
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

        match self.registry.upsert(src.into())? {
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
    fn paths(&self) -> Vec<AbsolutePath> {
        self.sources()
    }

    async fn handle(&mut self, events: Vec<FsEvent>) -> Result<(), HandlerError> {
        for event in events {
            let result = match event {
                FsEvent::Write(src) => match src.canonicalize() {
                    Ok(canon) => self.handle_write(canon).await,
                    Err(e) => {
                        debug!(?src, "write/create event for missing file; ignoring: {}", e);
                        Ok(())
                    }
                },
                FsEvent::Remove(src) => self.handle_remove(src),

                FsEvent::Move { from, to } => {
                    match to.canonicalize() {
                        Ok(new_canon) => self.handle_move(from, new_canon).await,
                        Err(e) => {
                            // Moved to path is missing. Treat as a delete of old file.
                            debug!(
                                ?to,
                                "move destination missing; downgrading to remove: {}", e
                            );
                            self.handle_remove(from)
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
        let root = AbsolutePath::new("/");

        let v = SecretFile::from_template("Db_Password".to_string(), "".to_string(), &root);
        assert_eq!(v.dest(), Path::new("/Db_Password"));

        let v = SecretFile::from_template("A/B/C".to_string(), "".to_string(), &root);
        assert_eq!(v.dest(), Path::new("/ABC"));

        let v = SecretFile::from_template("weird name".to_string(), "".to_string(), &root);
        assert_eq!(v.dest(), Path::new("/weird name"));

        let v = SecretFile::from_template("..//--__".to_string(), "".to_string(), &root);
        assert_eq!(v.dest(), Path::new("/..--__"));
    }

    #[test]
    fn test_size_parsing() {
        assert_eq!(MemSize::from_str("100").unwrap().bytes, 100);
        assert_eq!(MemSize::from_str("1k").unwrap().bytes, 1024);
        assert_eq!(MemSize::from_str("10M").unwrap().bytes, 10 * 1024 * 1024);
    }
}
