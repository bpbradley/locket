//! Secrets provider implementation
//!
//! Providers will inject secrets from templates
use anyhow::Result;
use clap::{Args, Subcommand};
use std::process::Command;

#[derive(Subcommand, Debug, Clone)]
pub enum ProviderSubcommand {
    /// 1Password
    Op(OpProvider),
}

impl ProviderSubcommand {
    /// Resolve provider from SECRETS_PROVIDER env
    /// Fails if the variable is unset or unsupported.
    pub fn from_env() -> Result<Self> {
        let kind = std::env::var("SECRETS_PROVIDER")
            .map_err(|_| anyhow::anyhow!("SECRETS_PROVIDER not set and no provider subcommand supplied; set SECRETS_PROVIDER=op or specify with cli"))?;
        match kind.to_ascii_lowercase().as_str() {
            "op" | "1password" | "1pass" => Ok(ProviderSubcommand::Op(OpProvider::default())),
            other => anyhow::bail!("unsupported SECRETS_PROVIDER '{other}'; supported: op"),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    #[error("provider command failed: {0}")]
    Failed(String),
}

/// Generic secrets provider exposing a rendering primitive and optional preparation.
pub trait SecretsProvider {
    /// Inject a template file at `src` to a materialized file at `dst`.
    fn inject(&self, src: &str, dst: &str) -> Result<(), ProviderError>;
    /// Perform any provider-specific preparation.
    fn prepare(&self) -> Result<(), ProviderError> {
        Ok(())
    }
}

impl SecretsProvider for ProviderSubcommand {
    fn prepare(&self) -> Result<(), ProviderError> {
        match self {
            ProviderSubcommand::Op(p) => p.prepare(),
        }
    }
    fn inject(&self, src: &str, dst: &str) -> Result<(), ProviderError> {
        match self {
            ProviderSubcommand::Op(p) => p.inject(src, dst),
        }
    }
}

/// 1Password `op`-based provider;
#[derive(Args, Debug, Clone, Default)]
pub struct OpProvider {
    /// 1Password service account token
    #[arg(
        long,
        env = "OP_SERVICE_ACCOUNT_TOKEN",
        hide_env_values = true,
        value_name = "TOKEN"
    )]
    pub token: Option<String>,
    /// Path to token file (used if --token absent)
    #[arg(long, env = "OP_SERVICE_ACCOUNT_TOKEN_FILE", value_name = "PATH")]
    pub token_file: Option<String>,
}

impl SecretsProvider for OpProvider {
    fn prepare(&self) -> Result<(), ProviderError> {
        if let Ok(v) = std::env::var("OP_SERVICE_ACCOUNT_TOKEN") {
            if !v.is_empty() {
                return Ok(());
            }
        }
        if let Some(t) = &self.token {
            if t.is_empty() {
                return Err(ProviderError::Failed("empty --token".into()));
            }
            std::env::set_var("OP_SERVICE_ACCOUNT_TOKEN", t);
            return Ok(());
        }
        if let Some(path) = &self.token_file {
            let token = std::fs::read_to_string(path)
                .map_err(|e| ProviderError::Failed(format!("read token file: {e}")))?
                .trim()
                .to_string();
            if token.is_empty() {
                return Err(ProviderError::Failed("token file empty".into()));
            }
            std::env::set_var("OP_SERVICE_ACCOUNT_TOKEN", &token);
            return Ok(());
        }

        Err(ProviderError::Failed(
            "OP_SERVICE_ACCOUNT_TOKEN not set".into(),
        ))
    }

    fn inject(&self, src: &str, dst: &str) -> Result<(), ProviderError> {
        let status = Command::new("op")
            .arg("inject")
            .arg("-i")
            .arg(src)
            .arg("-o")
            .arg(dst)
            .envs(std::env::vars())
            .status()
            .map_err(|e| ProviderError::Failed(e.to_string()))?;
        if status.success() {
            Ok(())
        } else {
            Err(ProviderError::Failed(format!(
                "status: {:?}",
                status.code()
            )))
        }
    }
}
