use crate::provider::SecretsProvider;
use crate::secrets::fs::SecretFs;
use crate::secrets::types::{InjectFailurePolicy, Injectable, SecretError, SecretValue};
use crate::secrets::utils;
use crate::template::Template;
use crate::write::FileWriter;
use clap::Args;
use std::borrow::Cow;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tracing::{debug, info, warn};

#[derive(Debug, Clone, Args)]
pub struct SecretsOpts {
    /// Mapping of source paths (holding secret templates)
    /// to destination paths (where secrets are materialized and reflected)
    /// in the form `SRC:DST` or `SRC=DST`. Multiple mappings can be
    /// provided, separated by commas, or supplied multiple times as arguments.
    /// e.g. `--map /templates:/run/secrets/locket/app --map /other_templates:/run/secrets/locket/other`
    #[arg(
        long = "map", 
        value_parser = parse_mapping,
        env = "SECRET_MAP", 
        value_delimiter = ',',
        default_value = "/templates:/run/secrets/locket",
        hide_env_values = true
    )]
    pub mapping: Vec<PathMapping>,
    /// Directory where secret values (literals) are materialized
    #[arg(
        long = "out",
        env = "VALUE_OUTPUT_DIR",
        default_value = "/run/secrets/locket",
        value_parser = parse_absolute,
    )]
    pub value_dir: PathBuf,
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
    #[arg(long = "max-file-size", env = "MAX_FILE_SIZE", default_value = "10M", value_parser = parse_size)]
    pub max_file_size: u64,
}

/// Filesystem events for SecretFs
#[derive(Debug, Ord, PartialOrd, Eq, PartialEq)]
pub enum FsEvent {
    Write(PathBuf),
    Remove(PathBuf),
    Move { from: PathBuf, to: PathBuf },
}

/// Mapping of source path to destination path for secret files
#[derive(Debug, Clone)]
pub struct PathMapping {
    src: PathBuf,
    dst: PathBuf,
}

impl PathMapping {
    pub fn new(src: impl AsRef<Path>, dst: impl AsRef<Path>) -> Self {
        Self {
            src: utils::clean(src),
            dst: utils::clean(dst),
        }
    }
    pub fn src(&self) -> &Path {
        &self.src
    }
    pub fn dst(&self) -> &Path {
        &self.dst
    }
}

impl Default for PathMapping {
    fn default() -> Self {
        Self::new("/templates", "/run/secrets/locket")
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
            destinations.push(m.dst());
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

impl Default for SecretsOpts {
    fn default() -> Self {
        Self {
            mapping: vec![PathMapping::default()],
            value_dir: PathBuf::from("/run/secrets/locket"),
            policy: InjectFailurePolicy::CopyUnmodified,
            max_file_size: 10 * 1024 * 1024,
        }
    }
}

#[derive(Debug, Clone, Args)]
pub struct SecretValues {
    /// Environment variables prefixed with this string will be treated as secret values
    #[arg(long, env = "VALUE_PREFIX", default_value = "secret_")]
    pub env_prefix: String,
    /// Additional secret values specified as LABEL=SECRET_TEMPLATE
    /// Multiple values can be provided, separated by semicolons.
    /// Or supplied multiple times as arguments.
    /// e.g. `--secret db_password={{op://vault/credentials/db_password}} --secret api_key={{op://vault/keys/api_key}}`
    #[arg(
        long = "secret",
        env = "SECRET_VALUE",
        value_name = "label={{template}}",
        value_delimiter = ';',
        hide_env_values = true
    )]
    pub values: Vec<String>,
}

impl SecretValues {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn load(&self) -> HashMap<String, String> {
        let mut map = HashMap::new();

        for (k, v) in std::env::vars() {
            if let Some(label) = k.strip_prefix(&self.env_prefix) {
                map.insert(label.to_string(), v);
            }
        }

        for s in &self.values {
            match s.split_once('=') {
                Some((k, v)) => {
                    map.insert(k.to_string(), v.to_string());
                }
                None => {
                    tracing::warn!("Ignoring malformed secret argument: '{}'", s);
                }
            }
        }

        map
    }
}

impl Default for SecretValues {
    fn default() -> Self {
        Self {
            env_prefix: "secret_".to_string(),
            values: Vec::new(),
        }
    }
}

pub struct Secrets {
    opts: SecretsOpts,
    fs: SecretFs,
    values: HashMap<String, SecretValue>,
    writer: FileWriter,
}

impl Secrets {
    pub fn new(opts: SecretsOpts) -> Self {
        let fs = SecretFs::new(opts.mapping.clone(), opts.max_file_size);
        Self {
            opts,
            fs,
            values: HashMap::new(),
            writer: FileWriter::default(),
        }
    }

    pub fn with_values(mut self, values: HashMap<String, impl AsRef<str>>) -> Self {
        for (label, template) in values {
            let v = value_source(&self.opts.value_dir, &label, template);
            self.values.insert(v.label.clone(), v);
        }
        self
    }

    pub fn with_writer(mut self, writer: FileWriter) -> Self {
        self.writer = writer;
        self
    }

    pub fn iter_values(&self) -> impl Iterator<Item = &SecretValue> {
        self.values.values()
    }

    pub fn options(&self) -> &SecretsOpts {
        &self.opts
    }

    pub fn add_value(&mut self, label: &str, template: impl AsRef<str>) -> &mut Self {
        let v = value_source(&self.opts.value_dir, label, template);
        self.values.insert(v.label.clone(), v);
        self
    }

    pub async fn try_inject(
        &self,
        item: &dyn Injectable,
        provider: &dyn SecretsProvider,
    ) -> Result<(), SecretError> {
        let content = item.content()?;

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
            .filter(|k| provider.accepts_key(k))
            .collect();

        if references.is_empty() {
            debug!(dst=?item.dst(), "no resolveable secrets found; passing through");
            self.writer.atomic_write(item.dst(), content.as_bytes())?;
            return Ok(());
        }

        info!(dst=?item.dst(), count=references.len(), "fetching secrets");
        let secrets_map = provider.fetch_map(&references).await?;

        let output = if has_keys {
            tpl.render(&secrets_map)
        } else {
            match secrets_map.get(content.trim()) {
                Some(val) => Cow::Borrowed(val.as_str()),
                None => {
                    warn!(dst=?item.dst(), "provider returned success but secret value was missing");
                    content
                }
            }
        };

        self.writer.atomic_write(item.dst(), output.as_bytes())?;

        Ok(())
    }

    pub async fn process(
        &self,
        item: &dyn Injectable,
        provider: &dyn SecretsProvider,
    ) -> Result<(), SecretError> {
        match self.try_inject(item, provider).await {
            Ok(_) => Ok(()),
            Err(e) => self.handle_policy(item, e, self.opts.policy),
        }
    }

    pub async fn inject_all(&self, provider: &dyn SecretsProvider) -> Result<(), SecretError> {
        // Combine sources
        let values = self.iter_values().map(|v| v as &dyn Injectable);
        let files = self.fs.iter_files().map(|f| f as &dyn Injectable);

        // TODO: Parallelize?
        for item in values.chain(files) {
            self.process(item, provider).await?;
        }
        Ok(())
    }

    fn handle_policy(
        &self,
        item: &dyn Injectable,
        err: SecretError,
        policy: InjectFailurePolicy,
    ) -> Result<(), SecretError> {
        match policy {
            InjectFailurePolicy::Error => Err(err),
            InjectFailurePolicy::CopyUnmodified => {
                warn!(
                    src = ?item.label(),
                    dst = ?item.dst(),
                    error = ?err,
                    "injection failed; policy=copy-unmodified. Reverting to raw copy."
                );
                let raw = item.content().unwrap_or(Cow::Borrowed(""));
                if !raw.is_empty() {
                    self.writer.atomic_write(item.dst(), raw.as_bytes())?;
                }
                Ok(())
            }
            InjectFailurePolicy::Ignore => {
                warn!(src = ?item.label(), dst = ?item.dst(), error = ?err, "injection failed; ignoring");
                Ok(())
            }
        }
    }

    pub fn collisions(&self) -> Result<(), SecretError> {
        // Collect all secret destinations and label their sources
        // to report in error messages.
        let mut entries: Vec<(&Path, String)> = Vec::new();

        for file in self.fs.iter_files() {
            entries.push((file.dst(), format!("File({:?})", file.src())));
        }

        for val in self.iter_values() {
            entries.push((val.dst(), format!("Value({})", val.label)));
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
                    blocked: next_src.clone(),
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
            let dst = file.dst();
            if dst.exists() {
                std::fs::remove_file(dst)?;
            }
            debug!("event: removed secret file: {:?}", file.dst());
        }

        // Attempt to bubble delete empty parent dirs up to the event implied ceiling.
        if let Some(ceiling) = self.fs.resolve(src) {
            let mut candidates = std::collections::HashSet::new();
            for file in &removed {
                if let Some(parent) = file.dst().parent() {
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

    async fn on_move(
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
            // TODO: I think this is overly conservative and will leave behind
            // empty dirs in some cases. I think parent and ceil_parent end up
            // being the same with this logic, so kind of pointless. I probably
            // need to slightly refactor bubble_delete idea, or I need to bubble
            // up to SecretFs root.
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

                    if from.exists() {
                        let _ = std::fs::remove_file(&from);
                    }
                }
            }
        }

        // Fallback to remove + write
        debug!(?old, ?new, "fallback move via remove + write");
        self.on_remove(old)?;
        self.on_write(provider, new).await?;

        Ok(())
    }

    async fn on_write(
        &mut self,
        provider: &dyn SecretsProvider,
        src: &Path,
    ) -> Result<(), SecretError> {
        if src.is_dir() {
            debug!(?src, "directory write event; scanning for children");
            let entries: Vec<PathBuf> = walkdir::WalkDir::new(src)
                .into_iter()
                .filter_map(|e| e.ok())
                .filter(|e| e.file_type().is_file())
                .map(|e| e.path().to_path_buf())
                .collect();

            for entry in entries {
                // Recursion.. Treat every child file as its own Write event.
                // Should only ever be one level deep here, since we are already
                // inside a directory write event.
                Box::pin(self.on_write(provider, &entry)).await?;
            }
            return Ok(());
        }
        // Tiny race condition here, if file is removed while we are processing it..
        // Only a possible issue with inject failure policy of Error.
        // Otherwise, this will lead to eventual consistency on the next processing event
        if src.exists() {
            self.fs.upsert(src);
            if let Some(file) = self.fs.iter_files().find(|f| f.src() == src) {
                self.process(file, provider).await?;
            }
        }
        Ok(())
    }

    pub async fn handle_fs_event(
        &mut self,
        provider: &dyn SecretsProvider,
        ev: FsEvent,
    ) -> Result<(), SecretError> {
        match ev {
            FsEvent::Write(src) => self.on_write(provider, &utils::clean(src)).await,
            FsEvent::Remove(src) => self.on_remove(&utils::clean(src)),
            FsEvent::Move { from, to } => {
                self.on_move(provider, &utils::clean(from), &utils::clean(to))
                    .await
            }
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

    Ok(PathMapping::new(src, dst))
}

/// Parse a human-friendly size string into bytes.
fn parse_size(s: &str) -> Result<u64, String> {
    let s = s.trim();

    // Find where the number ends and the suffix begins
    let digit_end = s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
    let (num_str, suffix) = s.split_at(digit_end);

    if num_str.is_empty() {
        return Err("No number provided".to_string());
    }

    let num: u64 = num_str
        .parse()
        .map_err(|e| format!("Invalid number: {}", e))?;

    let multiplier = match suffix.trim().to_ascii_lowercase().as_str() {
        "" | "b" | "byte" | "bytes" => 1,
        "k" | "kb" | "kib" => 1024,
        "m" | "mb" | "mib" => 1024 * 1024,
        "g" | "gb" | "gib" => 1024 * 1024 * 1024,
        _ => {
            return Err(format!(
                "Unknown size suffix: '{}'. Supported: k, m, g",
                suffix
            ));
        }
    };

    Ok(num.saturating_mul(multiplier))
}

fn parse_absolute(s: &str) -> Result<PathBuf, String> {
    Ok(utils::clean(s))
}

/// Construct a SecretValue from label + template.
fn value_source(output_root: &Path, label: &str, template: impl AsRef<str>) -> SecretValue {
    let sanitized = utils::sanitize_name(label);
    let dst = output_root.join(&sanitized);
    SecretValue::new(dst, template, sanitized)
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

    #[test]
    fn sanitize_basic() {
        assert_eq!(utils::sanitize_name("Db_Password"), "db_password");
        assert_eq!(utils::sanitize_name("A/B/C"), "a_b_c");
        assert_eq!(utils::sanitize_name("weird name"), "weird_name");
    }

    #[test]
    fn sanitize_unicode_and_symbols() {
        assert_eq!(utils::sanitize_name("πß?%"), "____");
        assert_eq!(utils::sanitize_name("..//--__"), "..__--__");
    }

    #[test]
    fn test_size_parsing() {
        assert_eq!(parse_size("100").unwrap(), 100);
        assert_eq!(parse_size("1k").unwrap(), 1024);
        assert_eq!(parse_size("1KB").unwrap(), 1024);
        assert_eq!(parse_size("10M").unwrap(), 10 * 1024 * 1024);
        assert_eq!(parse_size("1G").unwrap(), 1024 * 1024 * 1024);

        // Edge cases
        assert!(parse_size("").is_err());
        assert!(parse_size("mb").is_err());
        assert!(parse_size("10x").is_err());
    }
}
