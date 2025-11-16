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

/// File-backed secret
#[derive(Debug, Clone)]
pub struct SecretFile {
    pub dst: PathBuf,
}

/// Value-backed secret
#[derive(Debug, Clone)]
pub struct SecretValue {
    pub dst: PathBuf,
    pub template: String,
    pub label: String,
}

impl SecretValue {
    pub fn inject(
        &self,
        policy: InjectFailurePolicy,
        provider: &dyn SecretsProvider,
    ) -> Result<(), SecretError> {
        info!(dst=?self.dst, label=%self.label, "injecting value secret");
        let parent = self
            .dst
            .parent()
            .ok_or_else(|| SecretError::NoParent(self.dst.clone()))?;

        fs::create_dir_all(parent)?;

        let tmp_out = tempfile::Builder::new()
            .prefix(".tmp.")
            .tempfile_in(parent)?
            .into_temp_path();

        match provider.inject_from_bytes(self.template.as_bytes(), tmp_out.as_ref()) {
            Ok(()) => {
                write::atomic_move(tmp_out.as_ref(), &self.dst)?;
                Ok(())
            }
            Err(e) => match policy {
                InjectFailurePolicy::Error => Err(SecretError::InjectionFailed { source: e }),
                InjectFailurePolicy::CopyUnmodified => {
                    warn!(dst=?self.dst, label=%self.label, error=?e,
                          "injection failed; falling back to raw copy for value secret");
                    write::atomic_write(tmp_out.as_ref(), self.template.as_bytes())?;
                    write::atomic_move(tmp_out.as_ref(), &self.dst)?;
                    Ok(())
                }
                InjectFailurePolicy::Ignore => {
                    warn!(dst=?self.dst, label=%self.label, error=?e,
                          "injection failed; ignoring");
                    Ok(())
                }
            },
        }
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
                dir.files.insert(rel, SecretFile { dst });
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
                dir.files.insert(rel, SecretFile { dst });
                return true;
            }
        }
        false
    }

    pub fn rename_file(&mut self, old: &Path, new: &Path) -> bool {
        // Try to find a dir that owns `old`
        for (root, entry) in self.entries.iter_mut() {
            if let SecretEntry::Dir(dir) = entry
                && let Ok(old_rel) = old.strip_prefix(root)
            {
                let old_rel = old_rel.to_path_buf();
                if dir.files.remove(&old_rel).is_some() {
                    // If new still lives under the same dir, keep it, otherwise drop it.
                    if let Ok(new_rel) = new.strip_prefix(root) {
                        let new_rel = new_rel.to_path_buf();
                        let dst = dir.dst_root.join(&new_rel);
                        dir.files.insert(new_rel, SecretFile { dst });
                        return true;
                    } else {
                        // renamed out of this dir: treat as removal
                        return false;
                    }
                }
            }
        }

        // If we didn't find it, treat as just an upsert of the new path
        self.upsert_file(new)
    }

    pub fn remove_file(&mut self, src: &Path) -> Option<PathBuf> {
        // In the future, explicit File entries could be removed here first.

        // Look inside dirs
        for (root, entry) in self.entries.iter_mut() {
            if let SecretEntry::Dir(dir) = entry
                && let Ok(rel) = src.strip_prefix(root)
            {
                let rel = rel.to_path_buf();
                if let Some(file) = dir.files.remove(&rel) {
                    return Some(file.dst);
                }
            }
        }
        None
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
                inject_file_path(src, &file.dst, self.options.policy, provider)?;
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub fn inject_all(&self, provider: &dyn SecretsProvider) -> Result<(), SecretError> {
        for v in self.values.values() {
            v.inject(self.options.policy, provider)?;
        }

        for (root, entry) in &self.entries {
            match entry {
                SecretEntry::File(file) => {
                    inject_file_path(root, &file.dst, self.options.policy, provider)?;
                }
                SecretEntry::Dir(dir) => {
                    for (rel, file) in &dir.files {
                        let src = root.join(rel);
                        inject_file_path(&src, &file.dst, self.options.policy, provider)?;
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
        for (root, entry) in &self.entries {
            match entry {
                SecretEntry::File(f) => {
                    *counts.entry(f.dst.clone()).or_insert(0) += 1;
                }
                SecretEntry::Dir(dir) => {
                    for (rel, f) in &dir.files {
                        let _src = root.join(rel); // not used, but available if you log later
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
}

fn inject_file_path(
    src: &Path,
    dst: &Path,
    policy: InjectFailurePolicy,
    provider: &dyn SecretsProvider,
) -> Result<(), SecretError> {
    info!(src=?src, dst=?dst, "injecting file secret");
    let parent = dst
        .parent()
        .ok_or_else(|| SecretError::NoParent(dst.to_path_buf()))?;

    fs::create_dir_all(parent)?;

    let tmp_out = tempfile::Builder::new()
        .prefix(".tmp.")
        .tempfile_in(parent)?
        .into_temp_path();

    match provider.inject(src, tmp_out.as_ref()) {
        Ok(()) => {
            write::atomic_move(tmp_out.as_ref(), dst)?;
            Ok(())
        }
        Err(e) => match policy {
            InjectFailurePolicy::Error => Err(SecretError::InjectionFailed { source: e }),
            InjectFailurePolicy::CopyUnmodified => {
                warn!(src=?src, dst=?dst, error=?e,
                      "injection failed; falling back to raw copy for file secret");
                let bytes = fs::read(src)?;
                write::atomic_write(tmp_out.as_ref(), &bytes)?;
                write::atomic_move(tmp_out.as_ref(), dst)?;
                Ok(())
            }
            InjectFailurePolicy::Ignore => {
                warn!(src=?src, dst=?dst, error=?e, "injection failed; ignoring");
                Ok(())
            }
        },
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
