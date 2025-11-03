use crate::{config::Config, provider::SecretsProvider, write};
use anyhow::{Context, Result, anyhow};
use indexmap::IndexMap;
use std::fs;
use std::path::{Path, PathBuf};
use tempfile;
use tracing::{debug, info, warn};
use walkdir::WalkDir;

#[derive(Debug, Clone)]
pub struct ValueSource {
    pub dst: PathBuf,
    pub template: String,
    pub label: String,
}

#[derive(Default, Debug)]
pub struct Secrets {
    /// Mapping of template source -> destination
    pub files: IndexMap<PathBuf, PathBuf>,
    /// Value-based secrets (template string -> destination)
    pub values: Vec<ValueSource>,
}

impl Secrets {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_config(cfg: &Config) -> Result<Self> {
        let files = collect_files(&cfg.templates_dir, &cfg.output_dir);
        let values = collect_value_sources_from_env(&cfg.output_dir, "secret_");
        Ok(Self { files, values })
    }

    pub fn add_value(
        &mut self,
        output_root: &Path,
        label: &str,
        template: impl AsRef<str>,
    ) -> &mut Self {
        let vs = value_source(output_root, label, template);
        self.values.push(vs);
        self
    }

    pub fn extend_values_iter<L, T, I>(&mut self, output_root: &Path, pairs: I) -> &mut Self
    where
        L: AsRef<str>,
        T: AsRef<str>,
        I: IntoIterator<Item = (L, T)>,
    {
        for (label, tpl) in pairs.into_iter() {
            let vs = value_source(output_root, label.as_ref(), tpl);
            self.values.push(vs);
        }
        self
    }

    pub fn extend_values_from_env(&mut self, output_root: &Path, prefix: &str) -> &mut Self {
        let mut collected = collect_value_sources_from_env(output_root, prefix);
        self.values.append(&mut collected);
        self
    }

    /// Return destination paths that appear more than once across files + values
    pub fn collisions(&self) -> Vec<PathBuf> {
        use std::collections::HashMap;
        let mut counts: HashMap<PathBuf, usize> = HashMap::new();
        for dst in self.files.values() {
            *counts.entry(dst.clone()).or_insert(0) += 1;
        }
        for v in &self.values {
            *counts.entry(v.dst.clone()).or_insert(0) += 1;
        }
        counts
            .into_iter()
            .filter_map(|(p, c)| (c > 1).then_some(p))
            .collect()
    }

    pub fn inject_file(
        &self,
        cfg: &Config,
        provider: &dyn SecretsProvider,
        src: &Path,
    ) -> Result<bool> {
        let Some(dst) = self.files.get(src) else {
            return Ok(false);
        };

        info!(src=?src, dst=?dst, "injecting file secret");
        if let Some(parent) = dst.parent() {
            fs::create_dir_all(parent)?;
        }

        let tmp_out = tempfile::Builder::new()
            .prefix(".tmp.")
            .tempfile_in(dst.parent().ok_or_else(|| anyhow!("dst has no parent"))?)?
            .into_temp_path();

        match provider.inject(src, tmp_out.as_ref()) {
            Ok(()) => {
                write::atomic_move(tmp_out.as_ref(), dst)?;
                Ok(true)
            }
            Err(e) => {
                if cfg.inject_fallback_copy {
                    warn!(src=?src, dst=?dst, error=?e, "injection failed; falling back to raw copy for file secret");
                    let bytes =
                        fs::read(src).with_context(|| format!("read failed for {src:?}"))?;
                    write::atomic_write(tmp_out.as_ref(), &bytes)?;
                    write::atomic_move(tmp_out.as_ref(), dst)?;
                    Ok(true)
                } else {
                    Err(anyhow!("injection failed and fallback disabled: {e}"))
                }
            }
        }
    }

    /// Inject all secrets: values first, then files. Fallback copy semantics apply to both kinds.
    pub fn inject_all(&self, cfg: &Config, provider: &dyn SecretsProvider) -> Result<()> {
        // Values
        for v in &self.values {
            info!(dst=?v.dst, label=%v.label, "injecting value secret");
            if let Some(parent) = v.dst.parent() {
                fs::create_dir_all(parent)?;
            }
            let tmp_out = tempfile::Builder::new()
                .prefix(".tmp.")
                .tempfile_in(v.dst.parent().unwrap())?
                .into_temp_path();

            match provider.inject_from_bytes(v.template.as_bytes(), tmp_out.as_ref()) {
                Ok(()) => write::atomic_move(tmp_out.as_ref(), &v.dst)?,
                Err(e) if cfg.inject_fallback_copy => {
                    warn!(dst=?v.dst, label=%v.label, error=?e, "injection failed; falling back to raw copy for value secret");
                    write::atomic_write(tmp_out.as_ref(), v.template.as_bytes())?;
                    write::atomic_move(tmp_out.as_ref(), &v.dst)?;
                }
                Err(e) => return Err(anyhow!("injection failed and fallback disabled: {e}")),
            }
        }

        for src in self.files.keys() {
            self.inject_file(cfg, provider, src)?;
        }

        Ok(())
    }
}

pub fn collect_files(templates_root: &Path, output_root: &Path) -> IndexMap<PathBuf, PathBuf> {
    let mut map = IndexMap::new();
    if !templates_root.exists() {
        return map;
    }
    for entry in WalkDir::new(templates_root)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
    {
        let rel = match entry.path().strip_prefix(templates_root) {
            Ok(r) => r.to_path_buf(),
            Err(_) => continue,
        };
        let dst = output_root.join(rel);
        map.insert(entry.path().to_path_buf(), dst.clone());
        debug!(src=?entry.path(), dst=?dst, "collected file secret");
    }
    map
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
