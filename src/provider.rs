//! Secrets provider implementation
//!
//! Providers will inject secrets from templates
use anyhow::{Context, Result};
use clap::{Args, Subcommand};
use secrecy::{ExposeSecret, SecretString};
use std::{
    path::PathBuf,
    process::{Command, Stdio},
};
#[derive(Subcommand, Debug, Clone)]
pub enum Provider {
    /// 1Password
    Op(OpConfig),
}

impl Provider {
    pub fn build(self) -> Result<Box<dyn SecretsProvider>> {
        match self {
            Provider::Op(cfg) => Ok(Box::new(OpProvider::new(cfg)?)),
        }
    }

    /// Resolve from SECRETS_PROVIDER (for when no subcommand is provided)
    pub fn from_env() -> Result<Self> {
        let s = std::env::var("SECRETS_PROVIDER").context("no provider configured")?;
        match s.to_ascii_lowercase().as_str() {
            "op" | "1password" | "1pass" => Ok(Provider::Op(OpConfig::default())),
            other => anyhow::bail!("unsupported SECRETS_PROVIDER '{other}'; supported: op"),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    /// File/FS/process spawning errors
    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    /// External command errors
    #[error("command '{program}' failed with status {status:?}: {stderr}")]
    Exec {
        program: &'static str,
        status: Option<i32>,
        stderr: String,
    },

    /// Invalid or missing configuration
    #[error("invalid config: {0}")]
    InvalidConfig(String),

    /// Generic error
    #[error("{0}")]
    Other(String),
}

pub trait SecretsProvider {
    fn inject(&self, src: &str, dst: &str) -> Result<(), ProviderError>;
}

/// 1Password `op`-based configuration.
#[derive(Args, Debug, Clone, Default)]
pub struct OpConfig {
    /// Service account token (prefer file/env over inline)
    #[arg(long, env = "OP_SERVICE_ACCOUNT_TOKEN", hide_env_values = true)]
    pub token: Option<String>,

    /// Path to token file (used if --token absent)
    #[arg(long, env = "OP_SERVICE_ACCOUNT_TOKEN_FILE")]
    pub token_file: Option<PathBuf>,
}

impl OpConfig {
    fn resolve_token(&self) -> Result<SecretString> {
        if let Some(p) = &self.token_file {
            let s = std::fs::read_to_string(p)
                .with_context(|| format!("read token file {}", p.display()))?;
            let t = s.trim().to_owned();
            anyhow::ensure!(!t.is_empty(), "token file empty");
            return Ok(SecretString::new(t.into()));
        }
        if let Some(t) = &self.token {
            anyhow::ensure!(!t.is_empty(), "empty --token");
            return Ok(SecretString::new(t.clone().into()));
        }
        let t = std::env::var("OP_SERVICE_ACCOUNT_TOKEN")
            .context("OP_SERVICE_ACCOUNT_TOKEN not set")?;
        anyhow::ensure!(!t.is_empty(), "OP_SERVICE_ACCOUNT_TOKEN empty");
        Ok(SecretString::new(t.into()))
    }
}

pub struct OpProvider {
    token: SecretString,
}

impl OpProvider {
    pub fn new(cfg: OpConfig) -> Result<Self> {
        let token = cfg.resolve_token()?;
        Ok(Self { token })
    }
}

impl SecretsProvider for OpProvider {
    fn inject(&self, src: &str, dst: &str) -> Result<(), ProviderError> {
        let output = Command::new("op")
            .arg("inject")
            .arg("-i")
            .arg(src)
            .arg("-o")
            .arg(dst)
            .env_clear()
            .env("OP_SERVICE_ACCOUNT_TOKEN", self.token.expose_secret())
            .env("PATH", std::env::var("PATH").unwrap_or_default())
            .env("HOME", std::env::var("HOME").unwrap_or_default())
            .stderr(Stdio::piped())
            .stdout(Stdio::null())
            .output()?;

        if output.status.success() {
            Ok(())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
            Err(ProviderError::Exec {
                program: "op",
                status: output.status.code(),
                stderr,
            })
        }
    }
}
