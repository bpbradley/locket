//! Template mirroring: inject then fallback to copy when enabled

use crate::{config::Config, provider::SecretsProvider, write};
use anyhow::Context;
use rand::Rng;
use std::fs;
use std::path::{Path, PathBuf};
use tracing::{info, warn};
use walkdir::WalkDir;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemplatePlan {
    pub src: PathBuf,
    pub dst: PathBuf,
}

pub fn plan_templates(cfg: &Config) -> Vec<TemplatePlan> {
    let mut v = Vec::new();
    let src_root: &Path = &cfg.templates_dir;
    let out_root: &Path = &cfg.output_dir;
    if !src_root.exists() {
        return v;
    }
    for entry in WalkDir::new(src_root)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
    {
        let rel = entry.path().strip_prefix(src_root).unwrap().to_path_buf();
        let dst = out_root.join(rel);
        v.push(TemplatePlan {
            src: entry.path().to_path_buf(),
            dst,
        });
    }
    v
}

pub fn sync_templates(cfg: &Config, provider: &dyn SecretsProvider) -> anyhow::Result<()> {
    for plan in plan_templates(cfg) {
        let tmp_out = tmp_dest_path(&plan.dst);
        if let Some(parent) = plan.dst.parent() {
            fs::create_dir_all(parent)?;
        }
        // Try injection first
        let injected = provider.inject(
            plan.src.to_string_lossy().as_ref(),
            tmp_out.to_string_lossy().as_ref(),
        );
        match injected {
            Ok(()) => {
                info!(src=?plan.src, dst=?plan.dst, "template injected");
                write::atomic_move(&tmp_out, &plan.dst)?;
            }
            Err(e) => {
                if cfg.inject_fallback_copy {
                    warn!(src=?plan.src, dst=?plan.dst, "injection failed; falling back to raw copy");
                    // Raw copy to tmp then atomic move
                    let bytes = fs::read(&plan.src)
                        .with_context(|| format!("read failed for {:?}", plan.src))?;
                    write::atomic_write(&tmp_out, &bytes)?;
                    write::atomic_move(&tmp_out, &plan.dst)?;
                } else {
                    return Err(anyhow::anyhow!(
                        "injection failed and fallback disabled: {}",
                        e
                    ));
                }
            }
        }
    }
    Ok(())
}

pub(crate) fn tmp_dest_path(dst: &Path) -> PathBuf {
    let rand: String = rand::rng()
        .sample_iter(&rand::distr::Alphanumeric)
        .take(8)
        .map(char::from)
        .collect();
    let mut s = dst.as_os_str().to_owned();
    s.push(format!(".tmp.{}", rand));
    PathBuf::from(s)
}

/// Compute destination path for a given source under templates dir. Returns None if src is
/// not under the configured templates root.
pub fn dst_for_src(cfg: &Config, src: &Path) -> Option<PathBuf> {
    let src_root: &Path = &cfg.templates_dir;
    let out_root: &Path = &cfg.output_dir;
    let rel = src.strip_prefix(src_root).ok()?;
    Some(out_root.join(rel))
}

/// Sync a single template source file to its destination using inject-then-fallback logic.
pub fn sync_template_path(
    cfg: &Config,
    provider: &dyn SecretsProvider,
    src: &Path,
) -> anyhow::Result<()> {
    if !src.exists() || !src.is_file() {
        // Nothing to do for non-files
        return Ok(());
    }
    let Some(dst) = dst_for_src(cfg, src) else {
        return Ok(());
    };
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent)?;
    }

    let tmp_out = tmp_dest_path(&dst);
    // Try injection first
    match provider.inject(
        src.to_string_lossy().as_ref(),
        tmp_out.to_string_lossy().as_ref(),
    ) {
        Ok(()) => {
            write::atomic_move(&tmp_out, &dst)?;
            Ok(())
        }
        Err(e) => {
            if cfg.inject_fallback_copy {
                let bytes = fs::read(src)?;
                write::atomic_write(&tmp_out, &bytes)?;
                write::atomic_move(&tmp_out, &dst)?;
                Ok(())
            } else {
                Err(anyhow::anyhow!(
                    "injection failed and fallback disabled: {}",
                    e
                ))
            }
        }
    }
}

/// Remove the destination file that corresponds to a given source path, if it exists.
/// Does nothing if the src is outside the templates root or if the destination is absent.
pub fn remove_dst_for_src(cfg: &Config, src: &Path) -> anyhow::Result<()> {
    if let Some(dst) = dst_for_src(cfg, src)
        && dst.exists()
    {
        // Only try to remove files; directories are not managed here.
        if dst.is_file() {
            std::fs::remove_file(&dst)?;
        }
    }
    Ok(())
}
