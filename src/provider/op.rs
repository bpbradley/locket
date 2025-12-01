//! 1password (op) based provider implementation

use crate::provider::{AuthToken, ProviderError, SecretsProvider};
use async_trait::async_trait;
use clap::Args;
use secrecy::SecretString;
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Args, Debug, Clone, Default)]
pub struct OpConfig {
    /// 1Password token configuration
    #[command(flatten)]
    tok: OpToken,

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
    #[arg(
        long = "op.token",
        env = "OP_SERVICE_ACCOUNT_TOKEN",
        hide_env_values = true
    )]
    op_val: Option<SecretString>,

    /// Path to file containing 1Password service account token
    #[arg(long = "op.token-file", env = "OP_SERVICE_ACCOUNT_TOKEN_FILE")]
    op_file: Option<PathBuf>,
}

impl OpToken {
    pub fn resolve(&self) -> Result<AuthToken, ProviderError> {
        AuthToken::try_new(self.op_val.clone(), self.op_file.clone(), "op")
    }
}

pub struct OpProvider {
    #[allow(dead_code)] // TODO: this provider is just a stub for now with op cli removed
    token: AuthToken,
    #[allow(dead_code)]
    config: Option<PathBuf>,
}

impl OpProvider {
    pub fn new(cfg: OpConfig) -> Result<Self, ProviderError> {
        Ok(Self {
            token: cfg.tok.resolve()?,
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
