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

/// CLI / config options.
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


/// Filesystem events for SecretFs
#[derive(Debug, Ord, PartialOrd, Eq, PartialEq)]
pub enum FsEvent {
    Write (PathBuf),
    Remove (PathBuf),
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
        self.fs
            .collect_from_root(&self.opts.templates_root, &self.opts.output_root);

        let envs =
            collect_value_sources_from_env(&self.opts.output_root, &self.opts.env_value_prefix);
        for v in envs {
            self.values.insert(v.label.clone(), v);
        }

        self
    }

    pub fn add_value(&mut self, label: &str, template: impl AsRef<str>) -> &mut Self {
        let v = value_source(&self.opts.output_root, label, template);
        self.values.insert(v.label.clone(), v);
        self
    }

    pub fn extend_values(
        &mut self,
        pairs: impl IntoIterator<Item = (impl AsRef<str>, impl AsRef<str>)>,
    ) -> &mut Self {
        for (label, tpl) in pairs {
            let v = value_source(&self.opts.output_root, label.as_ref(), tpl.as_ref());
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

    fn on_removed(&mut self, src: &Path) -> Result<(), SecretError> {
        // If src is a directory, we *could* scan children here later.
        // For now, we only remove exact file matches.
        if src.is_dir() {
            debug!(
                ?src,
                "on_removed: directory removal; currently no subtree cleanup"
            );
            return Ok(());
        }

        if let Some(file) = self.fs.remove(src) {
            file.remove()?
        }
        Ok(())
    }

    /// Handle “renamed” fs event.
    fn on_renamed(
        &mut self,
        provider: &dyn SecretsProvider,
        old: &Path,
        new: &Path,
    ) -> Result<(), SecretError> {
        // Best effort: keep it simple for now.
        // try to drop old dst if we were tracking it.
        self.on_removed(old)?;

        // Then treat new as a fresh file.
        self.on_created_or_modified(provider, new)
    }

    fn on_created_or_modified(
        &mut self,
        provider: &dyn SecretsProvider,
        src: &Path,
    ) -> Result<(), SecretError> {
        // Only react to files, not directories.
        if src.is_dir() {
            debug!(?src, "on_created_or_modified: skipping directory");
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
            FsEvent::Write(src) => self.on_created_or_modified(provider, &src),
            FsEvent::Remove(src) => self.on_removed(&src),
            FsEvent::Move{ from, to } => self.on_renamed(provider, &from, &to),
        }
    }
}
