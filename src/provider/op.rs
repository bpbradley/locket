//! 1password (op) based provider implementation

use crate::provider::{AuthToken, ProviderError, SecretsProvider, macros::define_auth_token};
use async_trait::async_trait;
use clap::Args;
use futures::stream::{self, StreamExt};
use secrecy::ExposeSecret;
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use tokio::process::Command;

define_auth_token!(
    struct_name: OpToken,
    prefix: "op",
    env: "OP_SERVICE_ACCOUNT_TOKEN",
    group_id: "op_token",
    doc_string: "1Password Service Account token"
);

#[derive(Args, Debug, Clone, Default)]
pub struct OpConfig {
    /// 1Password token configuration
    #[command(flatten)]
    tok: OpToken,

    /// Optional: Path to 1Password config directory
    /// Defaults to standard op config locations if not provided,
    /// e.g. $XDG_CONFIG_HOME/op
    #[arg(long = "op.config-dir", env = "OP_CONFIG_DIR")]
    config_dir: Option<PathBuf>,
}

pub struct OpProvider {
    token: AuthToken,
    config: Option<PathBuf>,
}

impl OpProvider {
    pub async fn new(cfg: OpConfig) -> Result<Self, ProviderError> {
        let token: AuthToken = cfg.tok.try_into()?;

        // Try to authenticate with the provided token
        let mut cmd = Command::new("op");
        cmd.arg("whoami")
            .env("PATH", std::env::var("PATH").unwrap_or_default())
            .env("HOME", std::env::var("HOME").unwrap_or_default())
            .env(
                "XDG_CONFIG_HOME",
                std::env::var("XDG_CONFIG_HOME").unwrap_or_default(),
            )
            .env("OP_SERVICE_ACCOUNT_TOKEN", token.expose_secret())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let output = cmd.output().await.map_err(ProviderError::Io)?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(ProviderError::Unauthorized(format!(
                "op login failed: {}",
                stderr.trim()
            )));
        }

        Ok(Self {
            token,
            config: cfg.config_dir,
        })
    }
}

#[async_trait]
impl SecretsProvider for OpProvider {
    fn accepts_key(&self, key: &str) -> bool {
        key.starts_with("op://")
    }

    async fn fetch_map(
        &self,
        references: &[&str],
    ) -> Result<HashMap<String, String>, ProviderError> {
        const MAX_CONCURRENT_OPS: usize = 10;
        let refs: Vec<String> = references.iter().map(|s| s.to_string()).collect();

        let results: Vec<Result<Option<(String, String)>, ProviderError>> = stream::iter(refs)
            .map(|key| async move {
                let mut cmd = Command::new("op");
                cmd.arg("read")
                    .arg("--no-newline")
                    .arg(&key)
                    .env_clear()
                    .env("PATH", std::env::var("PATH").unwrap_or_default())
                    .env("HOME", std::env::var("HOME").unwrap_or_default())
                    .env(
                        "XDG_CONFIG_HOME",
                        std::env::var("XDG_CONFIG_HOME").unwrap_or_default(),
                    )
                    .env("OP_SERVICE_ACCOUNT_TOKEN", self.token.expose_secret())
                    .stdin(Stdio::null())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped());

                if let Some(path) = &self.config {
                    cmd.env("OP_CONFIG_DIR", path);
                }

                let output = cmd.output().await.map_err(ProviderError::Io)?;

                if output.status.success() {
                    let secret = String::from_utf8(output.stdout)
                        .map_err(|e| ProviderError::InvalidConfig(format!("utf8 error: {}", e)))?;
                    Ok(Some((key, secret)))
                } else {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    Err(ProviderError::Other(format!(
                        "op error for {}: {}",
                        key,
                        stderr.trim()
                    )))
                }
            })
            .buffer_unordered(MAX_CONCURRENT_OPS)
            .collect()
            .await;

        let mut map = HashMap::new();
        for res in results {
            match res {
                Ok(Some((k, v))) => {
                    map.insert(k, v);
                }
                Ok(None) => {}
                Err(e) => return Err(e),
            }
        }

        Ok(map)
    }
}
