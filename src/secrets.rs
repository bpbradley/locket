use crate::provider::ProviderError;
use crate::{provider::SecretsProvider, write};
use clap::{Args, ValueEnum};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::{env, fs};
use tempfile;
use thiserror::Error;
use tracing::{debug, info, warn};
use walkdir::WalkDir;

#[derive(Debug, Error)]
pub enum SecretError {
    #[error("provider: {0}")]
    Provider(#[from] crate::provider::ProviderError),

    #[error("injection failed: {source}")]
    InjectionFailed {
        #[source]
        source: crate::provider::ProviderError,
    },

    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("dst has no parent: {0}")]
    NoParent(std::path::PathBuf),
}

#[derive(Copy, Clone, Debug, ValueEnum, Default)]
pub enum InjectFailurePolicy {
    Error,
    #[default]
    CopyUnmodified,
    Ignore,
}

#[derive(Debug, Clone, Args, Default)]
pub struct SecretsOpts {
    #[arg(long, env = "TEMPLATES_ROOT", default_value = "/templates")]
    pub templates_root: PathBuf,
    #[arg(long, env = "OUTPUT_ROOT", default_value = "/run/secrets")]
    pub output_root: PathBuf,
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

impl SecretsOpts {
    pub fn build(&self) -> Result<Secrets, SecretError> {
        Ok(Secrets::new(self.clone()).collect())
    }
}

pub trait Injectable {
    fn label(&self) -> &str;
    fn dst(&self) -> &Path;
    fn copy(&self) -> Result<(), SecretError>;
    fn inject(
        &self,
        policy: InjectFailurePolicy,
        provider: &dyn SecretsProvider,
    ) -> Result<(), SecretError> {
        info!(src=?self.label(), dst=?self.dst(), "injecting secret");
        let parent = self
            .dst()
            .parent()
            .ok_or_else(|| SecretError::NoParent(self.dst().to_path_buf()))?;

        fs::create_dir_all(parent)?;

        let tmp_out = tempfile::Builder::new()
            .prefix(".tmp.")
            .tempfile_in(parent)?
            .into_temp_path();

        match self.injector(provider, tmp_out.as_ref()) {
            Ok(()) => {
                write::atomic_move(tmp_out.as_ref(), self.dst())?;
                Ok(())
            }
            Err(e) => match policy {
                InjectFailurePolicy::Error => Err(SecretError::InjectionFailed { source: e }),
                InjectFailurePolicy::CopyUnmodified => {
                    warn!(src=?self.label(), dst=?self.dst(), error=?e,
                        "injection failed; falling back to raw copy for file secret");
                    self.copy()?;
                    Ok(())
                }
                InjectFailurePolicy::Ignore => {
                    warn!(src=?self.label(), dst=?self.dst(), error=?e, "injection failed; ignoring");
                    Ok(())
                }
            },
        }
    }
    fn injector(&self, provider: &dyn SecretsProvider, dst: &Path) -> Result<(), ProviderError>;
}

/// File-backed secret
#[derive(Debug, Clone)]
pub struct SecretFile {
    /// Source template path
    pub src: PathBuf,
    /// Destination output path
    pub dst: PathBuf,
}

impl Injectable for SecretFile {
    fn label(&self) -> &str {
        self.src.to_str().unwrap_or("<invalid utf8>")
    }
    fn dst(&self) -> &Path {
        &self.dst
    }
    fn copy(&self) -> Result<(), SecretError> {
        write::atomic_copy(&self.src, &self.dst)?;
        Ok(())
    }
    fn injector(&self, provider: &dyn SecretsProvider, dst: &Path) -> Result<(), ProviderError> {
        provider.inject(&self.src, dst)?;
        Ok(())
    }
}

/// Value-backed secret
#[derive(Debug, Clone)]
pub struct SecretValue {
    pub dst: PathBuf,
    pub template: String,
    pub label: String,
}

impl Injectable for SecretValue {
    fn label(&self) -> &str {
        &self.label
    }
    fn dst(&self) -> &Path {
        &self.dst
    }
    fn copy(&self) -> Result<(), SecretError> {
        write::atomic_write(&self.dst, self.template.as_bytes())?;
        Ok(())
    }
    fn injector(&self, provider: &dyn SecretsProvider, dst: &Path) -> Result<(), ProviderError> {
        provider.inject_from_bytes(self.template.as_bytes(), dst)?;
        Ok(())
    }
}

/// A directory store of file-backed secrets
#[derive(Debug, Clone)]
pub struct SecretDir {
    /// Destination root for files under this dir
    pub dst_root: PathBuf,
    /// rel path (under src root) -> SecretFile
    pub files: HashMap<PathBuf, SecretFile>,
}

#[derive(Debug, Clone)]
pub enum SecretEntry {
    /// Watched template directory
    Dir(SecretDir),
    /// Watched explicit file
    File(SecretFile),
}

#[derive(Debug)]
pub enum FsEvent<'a> {
    CreatedOrModified { src: &'a Path },
    Removed { src: &'a Path },
    Renamed { old: &'a Path, new: &'a Path },
}

pub struct Secrets {
    options: SecretsOpts,
    entries: HashMap<PathBuf, SecretEntry>,
    values: HashMap<String, SecretValue>,
}

impl Secrets {
    pub fn new(options: SecretsOpts) -> Self {
        Self {
            options,
            entries: HashMap::new(),
            values: HashMap::new(),
        }
    }

    pub fn options(&self) -> &SecretsOpts {
        &self.options
    }

    pub fn collect(mut self) -> Self {
        self.entries.clear();
        self.values.clear();

        let templates_root = self.options.templates_root.clone();
        let output_root = self.options.output_root.clone();

        // For now, a single SecretDir from templates_root -> output_root
        let mut dir = SecretDir {
            dst_root: output_root.clone(),
            files: HashMap::new(),
        };

        for entry in WalkDir::new(&templates_root)
            .into_iter()
            .filter_map(|r| r.ok())
            .filter(|e| e.file_type().is_file())
        {
            let src = entry.path();
            if let Ok(rel) = src.strip_prefix(&templates_root) {
                let rel = rel.to_path_buf();
                let dst = output_root.join(&rel);
                debug!(src=?src, dst=?dst, "collected file secret");
                dir.files.insert(
                    rel,
                    SecretFile {
                        src: src.to_path_buf(),
                        dst,
                    },
                );
            }
        }

        // store the dir as a watched entry keyed by its src root
        self.entries.insert(templates_root, SecretEntry::Dir(dir));

        // collect values from env
        let envs = collect_value_sources_from_env(
            &self.options.output_root,
            &self.options.env_value_prefix,
        );
        for v in envs {
            self.values.insert(v.label.clone(), v);
        }

        self
    }

    pub fn add_value(&mut self, label: &str, template: impl AsRef<str>) -> &mut Self {
        let v = value_source(&self.options.output_root, label, template);
        self.values.insert(v.label.clone(), v);
        self
    }

    pub fn extend_values(
        &mut self,
        pairs: impl IntoIterator<Item = (impl AsRef<str>, impl AsRef<str>)>,
    ) -> &mut Self {
        for (label, tpl) in pairs {
            let v = value_source(&self.options.output_root, label.as_ref(), tpl.as_ref());
            self.values.insert(v.label.clone(), v);
        }
        self
    }

    pub fn upsert_file(&mut self, src: &Path) -> bool {
        // Future: explicit file entries would be handled here first.

        // For now: find a dir whose key is a prefix of src
        for (root, entry) in self.entries.iter_mut() {
            if let SecretEntry::Dir(dir) = entry
                && let Ok(rel) = src.strip_prefix(root)
            {
                let rel = rel.to_path_buf();
                let dst = dir.dst_root.join(&rel);
                dir.files.insert(
                    rel,
                    SecretFile {
                        src: src.to_path_buf(),
                        dst,
                    },
                );
                return true;
            }
        }
        false
    }

    /// Inject a single file by its source path
    pub fn inject_file(
        &self,
        provider: &dyn SecretsProvider,
        src: &Path,
    ) -> Result<bool, SecretError> {
        // Explicit file entries would be checked here first...

        // Dir-managed files
        for (root, entry) in &self.entries {
            if let SecretEntry::Dir(dir) = entry
                && let Ok(rel) = src.strip_prefix(root)
                && let Some(file) = dir.files.get(rel)
            {
                file.inject(self.options.policy, provider)?;
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub fn inject_all(&self, provider: &dyn SecretsProvider) -> Result<(), SecretError> {
        // value-backed secrets
        for v in self.values.values() {
            v.inject(self.options.policy, provider)?;
        }

        // file-backed secrets
        for entry in self.entries.values() {
            match entry {
                SecretEntry::File(file) => {
                    file.inject(self.options.policy, provider)?;
                }
                SecretEntry::Dir(dir) => {
                    for file in dir.files.values() {
                        file.inject(self.options.policy, provider)?;
                    }
                }
            }
        }
        Ok(())
    }

    pub fn collisions(&self) -> Vec<PathBuf> {
        use std::collections::HashMap;
        let mut counts: HashMap<PathBuf, usize> = HashMap::new();

        // values
        for v in self.values.values() {
            *counts.entry(v.dst.clone()).or_insert(0) += 1;
        }

        // files
        for entry in self.entries.values() {
            match entry {
                SecretEntry::File(f) => {
                    *counts.entry(f.dst.clone()).or_insert(0) += 1;
                }
                SecretEntry::Dir(dir) => {
                    for f in dir.files.values() {
                        *counts.entry(f.dst.clone()).or_insert(0) += 1;
                    }
                }
            }
        }

        counts
            .into_iter()
            .filter_map(|(p, n)| (n > 1).then_some(p))
            .collect()
    }

    fn take_file(&mut self, src: &Path) -> Option<SecretFile> {
        for (root, entry) in self.entries.iter_mut() {
            if let SecretEntry::Dir(dir) = entry
                && let Ok(rel) = src.strip_prefix(root)
            {
                let rel = rel.to_path_buf();
                if let Some(file) = dir.files.remove(&rel) {
                    return Some(file);
                }
            }
        }
        None
    }

    pub fn rename_file(&mut self, old: &Path, new: &Path) -> Result<bool, SecretError> {
        for (root, entry) in self.entries.iter_mut() {
            if let SecretEntry::Dir(dir) = entry
                && let Ok(old_rel) = old.strip_prefix(root)
            {
                let old_rel = old_rel.to_path_buf();

                // Is `new` still under this same root?
                let Ok(new_rel) = new.strip_prefix(root) else {
                    // Move out of this dir: delete the dst and forget it
                    if let Some(file) = dir.files.remove(&old_rel)
                        && file.dst.exists() {
                            fs::remove_file(&file.dst)?;
                        }
                    return Ok(false);
                };

                let new_rel = new_rel.to_path_buf();

                if let Some(mut file) = dir.files.remove(&old_rel) {
                    let old_dst = file.dst.clone();
                    file.src = new.to_path_buf();
                    file.dst = dir.dst_root.join(&new_rel);
                    let new_dst = file.dst.clone();

                    if let Some(parent) = new_dst.parent() {
                        fs::create_dir_all(parent)?;
                    }

                    // Try to move the actual secret file
                    fs::rename(&old_dst, &new_dst)?;

                    dir.files.insert(new_rel, file);
                    return Ok(true);
                } else {
                    return Ok(false);
                }
            }
        }

        Ok(false)
    }

    fn on_removed(&mut self, src: &Path) -> Result<(), SecretError> {
        if let Some(file) = self.take_file(src)
            && file.dst.exists()
                && let Err(e) = fs::remove_file(&file.dst) {
                    warn!(error=?e, dst=?file.dst, "failed to remove destination");
                    return Err(SecretError::Io(e));
                }
        Ok(())
    }

    fn on_renamed(
        &mut self,
        provider: &dyn SecretsProvider,
        old: &Path,
        new: &Path,
    ) -> Result<(), SecretError> {
        match self.rename_file(old, new)? {
            true => {
                // File was renamed within its own managed dir
                Ok(())
            }
            false => {
                // File was renamed outside of its managed dir and removed.
                // Reinject it (if possible)
                if self.upsert_file(new) {
                    let _ = self.inject_file(provider, new)?;
                }
                Ok(())
            }
        }
    }

    fn on_created_or_modified(
        &mut self,
        provider: &dyn SecretsProvider,
        src: &Path,
    ) -> Result<(), SecretError> {
        // If we accept this src into our model, inject it.
        if self.upsert_file(src) {
            self.inject_file(provider, src)?;
        }
        Ok(())
    }
    pub fn handle_fs_event(
        &mut self,
        provider: &dyn SecretsProvider,
        ev: FsEvent,
    ) -> Result<(), SecretError> {
        match ev {
            FsEvent::CreatedOrModified { src } => self.on_created_or_modified(provider, src),
            FsEvent::Removed { src } => self.on_removed(src),
            FsEvent::Renamed { old, new } => self.on_renamed(provider, old, new),
        }
    }
}

pub fn collect_value_sources<L, T, I>(output_root: &Path, pairs: I) -> Vec<SecretValue>
where
    I: IntoIterator<Item = (L, T)>,
    L: AsRef<str>,
    T: AsRef<str>,
{
    pairs
        .into_iter()
        .map(|(label, template)| value_source(output_root, label.as_ref(), template))
        .collect()
}

pub fn collect_value_sources_from_env(output_root: &Path, prefix: &str) -> Vec<SecretValue> {
    let stripped =
        env::vars().filter_map(|(k, v)| k.strip_prefix(prefix).map(|rest| (rest.to_string(), v)));
    collect_value_sources(output_root, stripped)
}

pub fn value_source(output_root: &Path, label: &str, template: impl AsRef<str>) -> SecretValue {
    let sanitized = sanitize_name(label);
    let dst = output_root.join(&sanitized);
    SecretValue {
        dst,
        template: template.as_ref().to_string(),
        label: sanitized,
    }
}

pub fn sanitize_name(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for ch in raw.chars() {
        let lc = ch.to_ascii_lowercase();
        if lc.is_ascii_lowercase() || lc.is_ascii_digit() || matches!(lc, '.' | '_' | '-' | '/') {
            out.push(lc);
        } else {
            out.push('_');
        }
    }
    out
}
