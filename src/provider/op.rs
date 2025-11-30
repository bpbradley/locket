//! 1password (op) based provider implementation

use crate::provider::{ProviderError, SecretsProvider};
use async_trait::async_trait;
use clap::Args;
use secrecy::{ExposeSecret, SecretString};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Args, Debug, Clone, Default)]
pub struct OpConfig {
    /// 1Password token configuration
    #[command(flatten)]
    token: OpToken,

    /// Optional: Path to 1Password config directory
    /// Defaults to standard op config locations if not provided,
    /// e.g. $XDG_CONFIG_HOME/op
    #[arg(long, env = "OP_CONFIG_DIR")]
    config_dir: Option<PathBuf>,
}

/// 1Password (op) based provider configuration
#[derive(Args, Debug, Clone, Default)]
#[group(id = "op_token", multiple = false, required = true)]
pub struct OpToken {
    /// 1Password service account token
    #[arg(long, env = "OP_SERVICE_ACCOUNT_TOKEN", hide_env_values = true)]
    token: Option<SecretString>,

    /// Path to file containing 1Password service account token
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

#[async_trait]
impl SecretsProvider for OpProvider {
    async fn fetch_map(
        &self,
        references: &[&str],
    ) -> Result<HashMap<String, String>, ProviderError> {
        // Simulated async fetch for demonstration purposes
        // Need to reimplement `op inject` using connect API
        let mut map = HashMap::new();

        for &key in references {
            // Simulate some async delay
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;

            let value = format!("SECRET[{}]", key);
            map.insert(key.to_string(), value);
        }

        Ok(map)
    }

    fn accepts_key(&self, key: &str) -> bool {
        key.starts_with("op://")
    }
}
