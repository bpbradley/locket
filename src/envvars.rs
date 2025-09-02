//! Env-sourced secrets implementation

use crate::{
    config::Config,
    provider::{SecretsProvider, ValueKind},
    write,
};
use anyhow::Context;
use rand::Rng;
use std::path::{Path, PathBuf};
use tempfile::NamedTempFile;
use tracing::{debug, info};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvSecret {
    pub name: String,
    pub dst: PathBuf,
    pub value: String,
}

pub fn plan_env_secrets(cfg: &Config) -> Vec<EnvSecret> {
    collect_env_secrets(cfg)
}

pub fn sync_env_secrets(cfg: &Config, provider: &dyn SecretsProvider) -> anyhow::Result<()> {
    let secrets = collect_env_secrets(cfg);
    for s in secrets {
        let kind = provider.classify_value(&s.value);
        if matches!(kind, ValueKind::DirectRef) {
            info!(path=?s.dst, "writing env secret via direct read");
            let bytes = provider
                .read(&s.value)
                .with_context(|| format!("provider read failed for {}", s.name))?;
            write::atomic_write(&s.dst, &bytes)
                .with_context(|| format!("atomic write failed for {:?}", s.dst))?;
        } else {
            info!(path=?s.dst, "writing env secret via template injection (inline value)");
            // Write a temporary template file with the inline value
            let mut tmpl = NamedTempFile::new().context("create temp template file")?;
            std::io::Write::write_all(&mut tmpl, s.value.as_bytes())?;
            let tmpl_path = tmpl.into_temp_path();

            // inject outputs to a temp file in the destination directory
            let tmp_out = tmp_dest_path(&s.dst);
            if let Some(parent) = s.dst.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let src_path: String = tmpl_path.as_os_str().to_string_lossy().into_owned();
            let dst_tmp: String = tmp_out.as_os_str().to_string_lossy().into_owned();
            provider
                .inject(&src_path, &dst_tmp)
                .with_context(|| format!("template injection failed for {}", s.name))?;
            write::atomic_move(&tmp_out, &s.dst)
                .with_context(|| format!("atomic move failed to {:?}", s.dst))?;
        }
    }
    Ok(())
}

fn collect_env_secrets(cfg: &Config) -> Vec<EnvSecret> {
    let mut out = Vec::new();
    for (k, v) in std::env::vars() {
        if let Some(rest) = k.strip_prefix("secret_") {
            let name = sanitize_name(rest);
            let dst = Path::new(&cfg.output_dir).join(&name);
            debug!(original=%k, sanitized=%name, path=?dst, "collected env secret");
            out.push(EnvSecret {
                name,
                dst,
                value: v,
            });
        }
    }
    out
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

fn tmp_dest_path(dst: &Path) -> PathBuf {
    let rand: String = rand::thread_rng()
        .sample_iter(&rand::distributions::Alphanumeric)
        .take(8)
        .map(char::from)
        .collect();
    let mut s = dst.as_os_str().to_owned();
    s.push(&format!(".tmp.{}", rand));
    PathBuf::from(s)
}
