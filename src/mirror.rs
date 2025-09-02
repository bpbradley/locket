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
    let src_root = Path::new(&cfg.templates_dir);
    let out_root = Path::new(&cfg.output_dir);
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
