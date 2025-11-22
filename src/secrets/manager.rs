use crate::provider::SecretsProvider;
use crate::secrets::fs::SecretFs;
use crate::secrets::types::{
    InjectFailurePolicy, Injectable, SecretError, SecretValue, collect_value_sources_from_env,
    value_source,
};
use clap::Args;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tracing::debug;

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
#[derive(Debug, Clone, Args, Default)]
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
        Ok(Secrets::new(self.clone()).collect())
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
        Self {
            opts,
            fs: SecretFs::new(),
            values: HashMap::new(),
        }
    }

    pub fn options(&self) -> &SecretsOpts {
        &self.opts
    }

    pub fn collect(mut self) -> Self {
        for mapping in &self.opts.mapping {
            self.fs.add_mapping(mapping);
        }

        let envs =
            collect_value_sources_from_env(&self.opts.value_dir, &self.opts.env_value_prefix);
        for v in envs {
            self.values.insert(v.label.clone(), v);
        }

        self
    }

    pub fn add_value(&mut self, label: &str, template: impl AsRef<str>) -> &mut Self {
        let v = value_source(&self.opts.value_dir, label, template);
        self.values.insert(v.label.clone(), v);
        self
    }

    pub fn extend_values(
        &mut self,
        pairs: impl IntoIterator<Item = (impl AsRef<str>, impl AsRef<str>)>,
    ) -> &mut Self {
        for (label, tpl) in pairs {
            let v = value_source(&self.opts.value_dir, label.as_ref(), tpl.as_ref());
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
        // If src is a directory, we *could* scan children here later.
        // For now, we only remove exact file matches.
        if src.is_dir() {
            debug!(
                ?src,
                "on_remove: directory removal; currently no subtree cleanup"
            );
            return Ok(());
        }

        if let Some(file) = self.fs.remove(src) {
            file.remove()?
        }
        Ok(())
    }

    fn on_move(
        &mut self,
        provider: &dyn SecretsProvider,
        old: &Path,
        new: &Path,
    ) -> Result<(), SecretError> {
        // Best effort: keep it simple for now.
        // try to drop old dst if we were tracking it.
        self.on_remove(old)?;

        // Then treat new as a fresh file.
        self.on_write(provider, new)
    }

    fn on_write(&mut self, provider: &dyn SecretsProvider, src: &Path) -> Result<(), SecretError> {
        // Only react to files, not directories.
        if src.is_dir() {
            debug!(?src, "on_write: skipping directory");
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
