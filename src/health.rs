use std::fs::{self, File};
use std::io::Write;

pub fn is_ready(path: &str) -> bool {
    std::path::Path::new(path).exists()
}

pub fn mark_ready(path: &str) -> anyhow::Result<()> {
    if let Some(parent) = std::path::Path::new(path).parent() {
        fs::create_dir_all(parent)?;
    }
    let mut f = File::create(path)?;
    f.write_all(b"ready")?;
    Ok(())
}
