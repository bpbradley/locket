//! Secrets provider implementation
//!
//! Providers can either read a direct reference (e.g., a provider-specific URI)
//! or inject a template file to a rendered output.
use anyhow::Result;
use clap::{Args, Subcommand};
use std::process::Command;

#[derive(Subcommand, Debug, Clone)]
pub enum ProviderSubcommand {
    /// 1Password
    Op(OpProvider),
}

impl ProviderSubcommand {
    pub fn from_env_or_default() -> Result<Self> {
        let kind = std::env::var("SECRETS_PROVIDER").unwrap_or_else(|_| "op".to_string());
        match kind.to_ascii_lowercase().as_str() {
            "op" | "1password" | "1pass" => Ok(ProviderSubcommand::Op(OpProvider::default())),
            other => anyhow::bail!("unsupported SECRETS_PROVIDER '{}'", other),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    #[error("provider command failed: {0}")]
    Failed(String),
}

/// Indicates how a provider wants a value handled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueKind {
    DirectRef,
    Template,
}

/// Generic secrets provider that can read secret references and inject templates.
pub trait SecretsProvider {
    /// Inject a template file at `src` to a fully-rendered file at `dst`.
    fn inject(&self, src: &str, dst: &str) -> Result<(), ProviderError>;
    /// Read the bytes for a provider-specific secret reference.
    fn read(&self, reference: &str) -> Result<Vec<u8>, ProviderError>;
    /// Perform any provider-specific preparation
    fn prepare(&self) -> Result<(), ProviderError> {
        Ok(())
    }
    /// Classify how to treat a raw value from env/template inputs.
    /// Default assumes it is a template requiring injection.
    fn classify_value(&self, s: &str) -> ValueKind {
        let _ = s;
        ValueKind::Template
    }
}

impl SecretsProvider for ProviderSubcommand {
    fn prepare(&self) -> Result<(), ProviderError> {
        match self {
            ProviderSubcommand::Op(p) => p.prepare(),
        }
    }
    fn classify_value(&self, s: &str) -> ValueKind {
        match self {
            ProviderSubcommand::Op(p) => p.classify_value(s),
        }
    }
    fn inject(&self, src: &str, dst: &str) -> Result<(), ProviderError> {
        match self {
            ProviderSubcommand::Op(p) => p.inject(src, dst),
        }
    }
    fn read(&self, reference: &str) -> Result<Vec<u8>, ProviderError> {
        match self {
            ProviderSubcommand::Op(p) => p.read(reference),
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
    fn classify_value(&self, s: &str) -> ValueKind {
        let t = s.trim();
        if t.starts_with("op://") && !t.contains("{{") && !t.contains("}}") {
            ValueKind::DirectRef
        } else {
            ValueKind::Template
        }
    }

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
            .envs(std::env::vars()) // env is filtered by container runtime
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

    fn read(&self, reference: &str) -> Result<Vec<u8>, ProviderError> {
        let output = Command::new("op")
            .arg("read")
            .arg(reference)
            .envs(std::env::vars())
            .output()
            .map_err(|e| ProviderError::Failed(e.to_string()))?;
        if output.status.success() {
            Ok(output.stdout)
        } else {
            Err(ProviderError::Failed(
                String::from_utf8_lossy(&output.stderr).to_string(),
            ))
        }
    }
}
