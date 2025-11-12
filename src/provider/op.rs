//! 1password (op) based provider implementation

use crate::provider::{ProviderError, SecretsProvider};
use clap::Args;
use secrecy::{ExposeSecret, SecretString};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// 1Password (op) based provider configuration
#[derive(Args, Debug, Clone, Default)]
#[group(id = "op_token", multiple = false, required = true)]
pub struct OpConfig {
    /// 1Password (op) service account token
    #[arg(long, env = "OP_SERVICE_ACCOUNT_TOKEN", hide_env_values = true)]
    pub token: Option<SecretString>,

    /// Path to file containing the service account token
    #[arg(long, env = "OP_SERVICE_ACCOUNT_TOKEN_FILE")]
    pub token_file: Option<PathBuf>,
}

impl OpConfig {
    pub fn resolve_token(&self) -> Result<SecretString, ProviderError> {
        match (&self.token, &self.token_file) {
            (Some(tok), None) => Ok(tok.clone()),
            (None, Some(path)) => {
                let txt = std::fs::read_to_string(path)?;
                let trimmed = txt.trim();
                if trimmed.is_empty() {
                    Err(ProviderError::InvalidConfig(format!(
                        "token file {} is empty",
                        path.display()
                    )))
                } else {
                    Ok(SecretString::new(trimmed.to_owned().into()))
                }
            }
            _ => Err(ProviderError::InvalidConfig(
                "missing credentials for op".into(),
            )),
        }
    }
}

pub struct OpProvider {
    token: SecretString,
}

impl OpProvider {
    pub fn new(cfg: OpConfig) -> Result<Self, ProviderError> {
        Ok(Self {
            token: cfg.resolve_token()?,
        })
    }
}

impl SecretsProvider for OpProvider {
    fn inject(&self, src: &Path, dst: &Path) -> Result<(), ProviderError> {
        let output = Command::new("op")
            .arg("inject")
            .arg("-i")
            .arg(src)
            .arg("-o")
            .arg(dst)
            .arg("--force")
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
            // keep logs sane; avoid massive stderr
            const MAX_ERR: usize = 8 * 1024;
            if stderr.len() > MAX_ERR {
                stderr.truncate(MAX_ERR);
                stderr.push_str("…[truncated]");
            }
            Err(ProviderError::Exec {
                program: "op",
                status: output.status.code(),
                stderr,
            })
        }
    }
}
