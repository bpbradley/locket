//! 1password (op) based provider implementation

use crate::provider::{ProviderError, SecretsProvider};
use clap::Args;
use secrecy::{ExposeSecret, SecretString};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[derive(Args, Debug, Clone)]
pub struct OpConfig {
    /// 1Password (op) token configuration
    #[command(flatten)]
    token: OpToken,

    /// Path to 1Password (op) config directory
    #[arg(long, env = "OP_CONFIG_DIR")]
    config_dir: Option<PathBuf>
}

impl Default for OpConfig {
    fn default() -> Self {
        Self {
            token: OpToken::default(),
            config_dir: None
        }
    }
}

/// 1Password (op) based provider configuration
#[derive(Args, Debug, Clone, Default)]
#[group(id = "op_token", multiple = false, required = true)]
pub struct OpToken {
    /// 1Password (op) service account token
    #[arg(long, env = "OP_SERVICE_ACCOUNT_TOKEN", hide_env_values = true)]
    token: Option<SecretString>,

    /// Path to file containing the service account token
    #[arg(long, env = "OP_SERVICE_ACCOUNT_TOKEN_FILE")]
    token_file: Option<PathBuf>,
}

impl OpToken {
    pub fn resolve(&self) -> Result<SecretString, ProviderError> {
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
    config: Option<PathBuf>,
}

impl OpProvider {
    pub fn new(cfg: OpConfig) -> Result<Self, ProviderError> {
        Ok(Self {
            token: cfg.token.resolve()?,
            config: cfg.config_dir,
        })
    }
}

impl SecretsProvider for OpProvider {
    fn inject(&self, src: &Path, dst: &Path) -> Result<(), ProviderError> {
        let mut cmd = Command::new("op");
        if let Some(config) = &self.config {
            cmd.arg("--config").arg(config);
        }
        let output = cmd
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
