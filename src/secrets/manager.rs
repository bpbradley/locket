use crate::provider::SecretsProvider;
use crate::secrets::fs::SecretFs;
use crate::secrets::path::{PathExt, PathMapping, parse_absolute};
use crate::secrets::types::{InjectFailurePolicy, Secret, SecretError, SecretFile, MemSize};
use crate::template::Template;
use crate::write::FileWriter;
use clap::Args;
use secrecy::ExposeSecret;
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
    #[arg(long = "max-file-size", env = "MAX_FILE_SIZE", default_value = "10M")]
    pub max_file_size: MemSize,
}

/// Filesystem events for SecretFs
#[derive(Debug, Ord, PartialOrd, Eq, PartialEq)]
pub enum FsEvent {
    Write(PathBuf),
    Remove(PathBuf),
    Move { from: PathBuf, to: PathBuf },
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
        self.value_dir = dir.as_ref().absolute();
        self
    }
    pub fn with_policy(mut self, policy: InjectFailurePolicy) -> Self {
        self.policy = policy;
        self
    }
    pub fn resolve(&mut self) -> Result<(), SecretError> {
        let mut sources = Vec::new();
        let mut destinations = Vec::new();

        for m in &mut self.mapping {
            // Enforce that all source paths exist at startup to avoid ambiguity on what this source is
            // This should already be enforced on the user input, but we double check just in case.
            // This will force the path to be canonicalized in a way that resolves symlinks and
            // requires that the path exists.
            m.resolve()?;
            sources.push(m.src());
            destinations.push(m.dst());
        }
        self.value_dir = self.value_dir.absolute();
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
            max_file_size: MemSize::default(),
        }
    }
}

pub struct SecretManager {
    opts: SecretsOpts,
    fs: SecretFs,
    values: HashMap<String, SecretFile>,
    writer: FileWriter,
}

impl SecretManager {
    pub fn new(opts: SecretsOpts) -> Self {
        let fs = SecretFs::new(opts.mapping.clone(), opts.max_file_size);
        Self {
            opts,
            fs,
            values: HashMap::new(),
            writer: FileWriter::default(),
        }
    }

    pub fn with_secrets(mut self, args: Vec<Secret>) -> Self {
        for arg in args {
            let key = arg.key.clone();
            let file = SecretFile::from_arg(arg, &self.opts.value_dir, self.opts.max_file_size);
            self.values.insert(key, file);
        }
        self
    }

    pub fn with_writer(mut self, writer: FileWriter) -> Self {
        self.writer = writer;
        self
    }

    pub fn iter_values(&self) -> impl Iterator<Item = &SecretFile> {
        self.values.values()
    }

    pub fn options(&self) -> &SecretsOpts {
        &self.opts
    }

    pub fn add_value(&mut self, label: &str, template: impl AsRef<str>) -> &mut Self {
        let v = SecretFile::from_template(
            label.to_string(),
            template.as_ref().to_string(),
            &self.opts.value_dir,
        );
        self.values.insert(label.to_string(), v);
        self
    }

    pub async fn try_inject(
        &self,
        file: &SecretFile,
        provider: &dyn SecretsProvider,
    ) -> Result<(), SecretError> {
        let content = file.content()?;

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
            debug!(dst=?file.dest(), "no resolveable secrets found; passing through");
            self.writer.atomic_write(file.dest(), content.as_bytes())?;
            return Ok(());
        }

        info!(dst=?file.dest(), count=references.len(), "fetching secrets");
        let secrets_map = provider.fetch_map(&references).await?;

        let output = if has_keys {
            tpl.render_with(|k| secrets_map.get(k).map(|s| s.expose_secret()))
        } else {
            match secrets_map.get(content.trim()) {
                Some(val) => Cow::Borrowed(val.expose_secret()),
                None => {
                    warn!(dst=?file.dest(), "provider returned success but secret value was missing");
                    content
                }
            }
        };

        self.writer.atomic_write(file.dest(), output.as_bytes())?;

        Ok(())
    }

    pub async fn process(
        &self,
        file: &SecretFile,
        provider: &dyn SecretsProvider,
    ) -> Result<(), SecretError> {
        match self.try_inject(file, provider).await {
            Ok(_) => Ok(()),
            Err(e) => self.handle_policy(file, e, self.opts.policy),
        }
    }

    pub async fn inject_all(&self, provider: &dyn SecretsProvider) -> Result<(), SecretError> {
        // Combine sources
        let values = self.iter_values();
        let files = self.fs.iter_files();

        for file in values.chain(files) {
            self.process(file, provider).await?;
        }
        Ok(())
    }

    fn handle_policy(
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
                // Attempt to read content again.
                // If it fails (e.g. file gone), unwrap_or handles it.
                let raw = file.content().unwrap_or(Cow::Borrowed(""));
                if !raw.is_empty() {
                    self.writer.atomic_write(file.dest(), raw.as_bytes())?;
                }
                Ok(())
            }
            InjectFailurePolicy::Ignore => {
                warn!(src = ?file.source().label(), dst = ?file.dest(), error = ?err, "injection failed; ignoring");
                Ok(())
            }
        }
    }

    pub fn collisions(&self) -> Result<(), SecretError> {
        // Collect all secret destinations and label their sources
        let mut entries: Vec<(&Path, String)> = Vec::new();

        for file in self.fs.iter_files() {
            entries.push((file.dest(), format!("File({:?})", file.source().label())));
        }

        for val in self.iter_values() {
            entries.push((val.dest(), format!("Value({})", val.source().label())));
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
            let dst = file.dest();
            if dst.exists() {
                std::fs::remove_file(dst)?;
            }
            debug!("event: removed secret file: {:?}", file.dest());
        }

        // Attempt to bubble delete empty parent dirs up to the event implied ceiling.
        if let Some(ceiling) = self.fs.resolve(src) {
            let mut candidates = std::collections::HashSet::new();
            for file in &removed {
                if let Some(parent) = file.dest().parent() {
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
                Err(e) if e.kind() == std::io::ErrorKind::DirectoryNotEmpty => break,
                Err(_) => break,
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

            match std::fs::rename(&from, &to) {
                Ok(_) => {
                    debug!(?old, ?new, "moved");
                    if let Some(parent) = from.parent()
                        && let Some(ceiling) = self.fs.resolve(old)
                        && let Some(ceil_parent) = ceiling.parent()
                        && parent.starts_with(ceil_parent)
                    {
                        self.bubble_delete(parent.to_path_buf(), ceil_parent);
                    }

                    return Ok(());
                }
                Err(e) => {
                    warn!(error=?e, "move failed; falling back to reinjection");
                    self.fs.remove(new); // Rollback state
                    if from.exists() {
                        let _ = std::fs::remove_file(&from);
                    }
                }
            }
        }

        // Fallback
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
                Box::pin(self.on_write(provider, &entry)).await?;
            }
            return Ok(());
        }

        match self.fs.upsert(src)? {
            Some(file) => {
                self.process(&file, provider).await?;
            }
            None => {
                // File ignored
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
            FsEvent::Write(src) => self.on_write(provider, &src.clean()).await,
            FsEvent::Remove(src) => self.on_remove(&src.clean()),
            FsEvent::Move { from, to } => self.on_move(provider, &from.clean(), &to.clean()).await,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use std::str::FromStr;
    use crate::secrets::types::MemSize;

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
