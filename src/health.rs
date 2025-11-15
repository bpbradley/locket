use clap::Args;
use std::path::PathBuf;
#[derive(Args, Debug)]
pub struct StatusFile {
    /// Status file path
    #[arg(
        long = "status-file",
        env = "STATUS_FILE",
        default_value = "/tmp/.secret-sidecar/ready"
    )]
    pub path: PathBuf,
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
}
