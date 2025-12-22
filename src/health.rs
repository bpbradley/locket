//! Healthcheck probe for the `locket run` sidecar service.
//!
//! The health is determined by the presence of a "ready" status file,
//! which is created when all secrets have been successfully materialized.
//! If the file is absent, the sidecar is considered unhealthy.
use crate::path::parse_absolute;
use clap::Args;
use std::path::PathBuf;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum HealthError {
    #[error("service is unhealthy: status file not found")]
    Unhealthy,

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

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
    pub fn mark_ready(&self) -> std::io::Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&self.path, b"ready")?;
        Ok(())
    }
    pub fn clear(&self) -> std::io::Result<()> {
        if self.path.exists() {
            std::fs::remove_file(&self.path)?;
        }
        Ok(())
    }
}
