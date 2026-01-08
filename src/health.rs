//! Healthcheck probe for the `locket inject` sidecar service.
//!
//! The health is determined by the presence of a "ready" status file,
//! which is created when all secrets have been successfully materialized.
//! If the file is absent, the sidecar is considered unhealthy.
use crate::path::AbsolutePath;
use serde::Deserialize;
use std::str::FromStr;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum HealthError {
    #[error("service is unhealthy: status file not found")]
    Unhealthy,

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[serde(try_from = "String")]
pub struct StatusFile(AbsolutePath);

impl Default for StatusFile {
    fn default() -> Self {
        #[cfg(target_os = "linux")]
        return StatusFile(AbsolutePath::new("/dev/shm/locket/ready"));
        #[cfg(target_os = "macos")]
        return StatusFile(AbsolutePath::new("/private/tmp/locket/ready"));
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        return StatusFile(AbsolutePath::new("./locket-ready"));
    }
}

impl TryFrom<String> for StatusFile {
    type Error = String;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        Ok(Self(s.try_into()?))
    }
}

impl FromStr for StatusFile {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(StatusFile(AbsolutePath::from_str(s)?))
    }
}

impl std::fmt::Display for StatusFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0.display())
    }
}

impl StatusFile {
    pub fn new(path: AbsolutePath) -> Self {
        Self(path)
    }
    pub fn is_ready(&self) -> bool {
        self.0.exists()
    }
    pub fn mark_ready(&self) -> std::io::Result<()> {
        if let Some(parent) = self.0.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&self.0, b"ready")?;
        Ok(())
    }
    pub fn clear(&self) -> std::io::Result<()> {
        if self.0.exists() {
            std::fs::remove_file(&self.0)?;
        }
        Ok(())
    }
}
