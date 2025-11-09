use crate::{provider::SecretsProvider, write};
use clap::{Args, ValueEnum};
use indexmap::IndexMap;
use std::fs;
use std::path::{Path, PathBuf};
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

    #[error("injection failed and fallback disabled")]
    FallbackDisabled {
        #[source]
        cause: Box<SecretError>,
    },
}

#[derive(Copy, Clone, Debug, ValueEnum, Default)]
pub enum InjectFailurePolicy {
    Error,
    #[default]
    CopyUnmodified,
    Ignore,
}

#[derive(Debug, Clone, Args, Default)]
pub struct SecretsConfig {
    #[arg(long, env = "TEMPLATES_DIR", default_value = "/templates")]
    pub templates_dir: PathBuf,
    #[arg(long, env = "OUTPUT_DIR", default_value = "/run/secrets")]
    pub output_dir: PathBuf,
    #[arg(long, env = "VALUE_PREFIX", default_value = "secret_")]
    pub env_value_prefix: String,
    #[arg(long, env = "INJECT_FAILURE_POLICY", value_enum, default_value_t = InjectFailurePolicy::CopyUnmodified)]
    pub inject_failure_policy: InjectFailurePolicy,
}

#[derive(Debug, Clone)]
pub struct FileSource {
    pub src: PathBuf,
    pub dst: PathBuf,
}

impl FileSource {
    pub fn from_src(templates_root: &Path, output_root: &Path, src: PathBuf) -> Option<Self> {
        let rel = src.strip_prefix(templates_root).ok()?.to_owned();
        Some(Self {
            src,
            dst: output_root.join(rel),
        })
    }

    pub fn rename(&mut self, templates_root: &Path, output_root: &Path, new_src: PathBuf) -> bool {
        match new_src.strip_prefix(templates_root) {
            Ok(rel) => {
                let rel = rel.to_owned();
                self.src = new_src;
                self.dst = output_root.join(rel);
                true
            }
            Err(_) => false,
        }
    }

    pub fn inject(
        &self,
        policy: InjectFailurePolicy,
        provider: &dyn SecretsProvider,
    ) -> Result<(), SecretError> {
        info!(src=?self.src, dst=?self.dst, "injecting file secret");
        if let Some(parent) = self.dst.parent() {
            fs::create_dir_all(parent)?;
        }
        let parent = self
            .dst
            .parent()
            .ok_or_else(|| SecretError::NoParent(self.dst.clone()))?;
        let tmp_out = tempfile::Builder::new()
            .prefix(".tmp.")
            .tempfile_in(parent)?
            .into_temp_path();

        match provider.inject(&self.src, tmp_out.as_ref()) {
            Ok(()) => {
                write::atomic_move(tmp_out.as_ref(), &self.dst)?;
                Ok(())
            }
            Err(e) => match policy {
                InjectFailurePolicy::Error => Err(SecretError::InjectionFailed { source: e }),
                InjectFailurePolicy::CopyUnmodified => {
                    warn!(src=?self.src, dst=?self.dst, error=?e, "injection failed; falling back to raw copy for file secret");
                    let bytes = fs::read(&self.src)?;
                    write::atomic_write(tmp_out.as_ref(), &bytes)?;
                    write::atomic_move(tmp_out.as_ref(), &self.dst)?;
                    Ok(())
                }
                InjectFailurePolicy::Ignore => {
                    warn!(src=?self.src, dst=?self.dst, error=?e, "injection failed; ignoring");
                    Ok(())
                }
            },
        }
    }
}

#[derive(Debug, Clone)]
pub struct ValueSource {
    pub dst: PathBuf,
    pub template: String,
    pub label: String,
}

impl ValueSource {
    pub fn inject(
        &self,
        policy: InjectFailurePolicy,
        provider: &dyn SecretsProvider,
    ) -> Result<(), SecretError> {
        info!(dst=?self.dst, label=%self.label, "injecting value secret");
        if let Some(parent) = self.dst.parent() {
            fs::create_dir_all(parent)?;
        }
        let parent = self
            .dst
            .parent()
            .ok_or_else(|| SecretError::NoParent(self.dst.clone()))?;
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
                    warn!(dst=?self.dst, label=%self.label, error=?e, "injection failed; falling back to raw copy for value secret");
                    write::atomic_write(tmp_out.as_ref(), self.template.as_bytes())?;
                    write::atomic_move(tmp_out.as_ref(), &self.dst)?;
                    Ok(())
                }
                InjectFailurePolicy::Ignore => {
                    warn!(dst=?self.dst, label=%self.label, error=?e, "injection failed; ignoring");
                    Ok(())
                }
            },
        }
    }
}

#[derive(Debug, Clone)]
pub enum SecretItem {
    File(FileSource),
    Value(ValueSource),
}

impl SecretItem {
    #[inline]
    pub fn dst(&self) -> &Path {
        match self {
            SecretItem::File(f) => &f.dst,
            SecretItem::Value(v) => &v.dst,
        }
    }

    #[inline]
    pub fn src_path(&self) -> Option<&Path> {
        match self {
            SecretItem::File(f) => Some(&f.src),
            SecretItem::Value(_) => None,
        }
    }

    pub fn inject(
        &self,
        policy: InjectFailurePolicy,
        provider: &dyn SecretsProvider,
    ) -> Result<(), SecretError> {
        match self {
            SecretItem::File(f) => f.inject(policy, provider),
            SecretItem::Value(v) => v.inject(policy, provider),
        }
    }
}

#[derive(Debug)]
pub struct Secrets {
    pub templates_root: PathBuf,
    pub output_root: PathBuf,
    pub policy: InjectFailurePolicy,

    items: Vec<Option<SecretItem>>,
    file_index: IndexMap<PathBuf, usize>,
}

impl Secrets {
    pub fn new(templates_root: PathBuf, output_root: PathBuf, policy: InjectFailurePolicy) -> Self {
        Self {
            templates_root,
            output_root,
            policy,
            items: Vec::new(),
            file_index: IndexMap::new(),
        }
    }

    pub fn from_config(cfg: &SecretsConfig) -> Result<Self, SecretError> {
        let mut s = Self::new(
            cfg.templates_dir.clone(),
            cfg.output_dir.clone(),
            cfg.inject_failure_policy,
        );
        for fs in collect_files_iter(&s.templates_root.clone(), &s.output_root.clone()) {
            s.push_file(fs);
        }
        s.extend_values_from_env(&cfg.env_value_prefix);
        Ok(s)
    }

    pub fn add_value(&mut self, label: &str, template: impl AsRef<str>) -> &mut Self {
        self.push_value(value_source(&self.output_root, label, template));
        self
    }

    pub fn extend_values(
        &mut self,
        pairs: impl IntoIterator<Item = (impl AsRef<str>, impl AsRef<str>)>,
    ) -> &mut Self {
        for (label, tpl) in pairs {
            self.push_value(value_source(
                &self.output_root,
                label.as_ref(),
                tpl.as_ref(),
            ));
        }
        self
    }

    pub fn extend_values_from_env(&mut self, prefix: &str) -> &mut Self {
        for v in collect_value_sources_from_env(&self.output_root, prefix) {
            self.push_value(v);
        }
        self
    }

    pub fn upsert_file(&mut self, src: PathBuf) -> bool {
        if let Some(newf) =
            FileSource::from_src(&self.templates_root, &self.output_root, src.clone())
        {
            if let Some(&idx) = self.file_index.get(&src) {
                self.items[idx] = Some(SecretItem::File(newf));
            } else {
                self.push_file(newf);
            }
            true
        } else {
            false
        }
    }

    pub fn rename_file(&mut self, old_src: PathBuf, new_src: PathBuf) -> bool {
        let Some(idx) = self.file_index.swap_remove(&old_src) else {
            return self.upsert_file(new_src);
        };

        match self.items.get_mut(idx) {
            Some(Some(SecretItem::File(f))) => {
                if f.rename(&self.templates_root, &self.output_root, new_src.clone()) {
                    self.file_index.insert(new_src, idx);
                    true
                } else {
                    self.items[idx] = None;
                    false
                }
            }
            _ => {
                self.items[idx] = None;
                false
            }
        }
    }

    pub fn remove_file(&mut self, src: &Path) -> Option<PathBuf> {
        let idx = self.file_index.swap_remove(src)?;
        if let Some(slot) = self.items.get_mut(idx)
            && let Some(SecretItem::File(f)) = slot.as_ref()
        {
            let dst = f.dst.clone();
            *slot = None;
            return Some(dst);
        }
        None
    }

    pub fn inject_file(
        &self,
        provider: &dyn SecretsProvider,
        src: &Path,
    ) -> Result<bool, SecretError> {
        if let Some(&idx) = self.file_index.get(src)
            && let Some(Some(item)) = self.items.get(idx)
        {
            item.inject(self.policy, provider)?;
            return Ok(true);
        }
        Ok(false)
    }

    pub fn inject_all(&self, provider: &dyn SecretsProvider) -> Result<(), SecretError> {
        for item in self.items.iter().flatten() {
            if let SecretItem::Value(v) = item {
                v.inject(self.policy, provider)?;
            }
        }
        for item in self.items.iter().flatten() {
            if let SecretItem::File(f) = item {
                f.inject(self.policy, provider)?;
            }
        }
        Ok(())
    }

    pub fn collisions(&self) -> Vec<PathBuf> {
        use std::collections::HashMap;
        let mut counts: HashMap<PathBuf, usize> = HashMap::new();

        for item in self.items.iter().flatten() {
            *counts.entry(item.dst().to_path_buf()).or_insert(0) += 1;
        }

        counts
            .into_iter()
            .filter_map(|(p, n)| (n > 1).then_some(p))
            .collect()
    }

    fn push_file(&mut self, f: FileSource) {
        let idx = self.items.len();
        self.file_index.insert(f.src.clone(), idx);
        self.items.push(Some(SecretItem::File(f)));
    }

    fn push_value(&mut self, v: ValueSource) {
        self.items.push(Some(SecretItem::Value(v)));
    }
}

pub fn collect_files_iter<'a>(
    templates_root: &'a Path,
    output_root: &'a Path,
) -> impl Iterator<Item = FileSource> + 'a {
    WalkDir::new(templates_root)
        .into_iter()
        .filter_map(|r| r.ok())
        .filter(|e| e.file_type().is_file())
        .filter_map(move |e| {
            let src = e.path().to_path_buf();
            FileSource::from_src(templates_root, output_root, src).inspect(|fs| {
                debug!(src=?fs.src, dst=?fs.dst, "collected file secret");
            })
        })
}

pub fn collect_value_sources<L, T, I>(output_root: &Path, pairs: I) -> Vec<ValueSource>
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

pub fn collect_value_sources_from_env(output_root: &Path, prefix: &str) -> Vec<ValueSource> {
    let stripped = std::env::vars()
        .filter_map(|(k, v)| k.strip_prefix(prefix).map(|rest| (rest.to_string(), v)));
    collect_value_sources(output_root, stripped)
}

pub fn value_source(output_root: &Path, label: &str, template: impl AsRef<str>) -> ValueSource {
    let sanitized = sanitize_name(label);
    let dst = output_root.join(&sanitized);
    ValueSource {
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
