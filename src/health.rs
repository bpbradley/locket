use crate::secrets::parse_absolute;
use clap::Args;
use std::path::PathBuf;
#[derive(Args, Debug)]
pub struct StatusFile {
    /// Status file path used for healthchecks
    #[arg(
        long = "status-file",
        env = "LOCKET_STATUS_FILE",
        default_value = "/tmp/.locket/ready",
        value_parser = parse_absolute,
    )]
    path: PathBuf,
}

impl StatusFile {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }
    pub fn is_ready(&self) -> bool {
        self.path.exists()
    }
    pub fn mark_ready(&self) -> anyhow::Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&self.path, b"ready")?;
        Ok(())
    }
    pub fn clear(&self) -> anyhow::Result<()> {
        if self.path.exists() {
            std::fs::remove_file(&self.path)?;
        }
        Ok(())
    }
}
