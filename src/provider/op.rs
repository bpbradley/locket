//! 1password (op) based provider implementation
//!
use anyhow::{Context, Result};
use clap::Args;
use secrecy::{ExposeSecret, SecretString};
use std::path::PathBuf;
use std::process::{Command as ProcCommand, Stdio};

/// 1Password provider configuration
#[derive(Args, Debug, Clone, Default)]
#[group(id = "op_token", multiple = false, required = true)]
pub struct OpConfig {
    /// 1Password service account token
    #[arg(long, env = "OP_SERVICE_ACCOUNT_TOKEN", hide_env_values = true)]
    pub token: Option<SecretString>,

    /// Path to file containing the service account token
    #[arg(long, env = "OP_SERVICE_ACCOUNT_TOKEN_FILE")]
    pub token_file: Option<PathBuf>,
}

impl OpConfig {
    pub fn resolve_token(&self) -> Result<SecretString> {
        if let Some(p) = &self.token_file {
            let t = std::fs::read_to_string(p)
                .with_context(|| format!("read token file {}", p.display()))?
                .trim()
                .to_owned();
            anyhow::ensure!(!t.is_empty(), "token file empty");
            return Ok(SecretString::new(t.into()));
        }
        if let Some(t) = &self.token {
            return Ok(t.clone());
        }
        let t = std::env::var("OP_SERVICE_ACCOUNT_TOKEN")
            .context("OP_SERVICE_ACCOUNT_TOKEN not set")?;
        anyhow::ensure!(!t.is_empty(), "OP_SERVICE_ACCOUNT_TOKEN empty");
        Ok(SecretString::new(t.into()))
    }
}

/// Runtime `op` provider.
pub struct OpProvider {
    token: SecretString,
}

impl OpProvider {
    pub fn new(cfg: OpConfig) -> Result<Self> {
        Ok(Self {
            token: cfg.resolve_token()?,
        })
    }
}

impl crate::provider::SecretsProvider for OpProvider {
    fn inject(&self, src: &str, dst: &str) -> Result<(), crate::provider::ProviderError> {
        let output = ProcCommand::new("op")
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
            let mut stderr = String::from_utf8_lossy(&output.stderr).to_string();
            // keep logs sane; avoid massive stderr spew
            const MAX_ERR: usize = 8 * 1024;
            if stderr.len() > MAX_ERR {
                stderr.truncate(MAX_ERR);
                stderr.push_str("…[truncated]");
            }
            Err(crate::provider::ProviderError::Exec {
                program: "op",
                status: output.status.code(),
                stderr,
            })
        }
    }
}
