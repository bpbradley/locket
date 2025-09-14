use std::path::Path;

pub fn is_ready(path: impl AsRef<Path>) -> bool {
    path.as_ref().exists()
}

pub fn mark_ready(path: impl AsRef<Path>) -> anyhow::Result<()> {
    let p = path.as_ref();
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(p, b"ready")?;
    Ok(())
}
