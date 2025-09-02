//! Secrets provider implementation
//!
//! Providers can either read a direct reference (e.g., a provider-specific URI)
//! or inject a template file to a rendered output.

use crate::config::Config;
use std::process::Command;

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
    /// Perform any provider-specific preparation (env validation, tokens, etc.).
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

/// 1Password `op`-based provider;
#[derive(Debug, Clone, Default)]
pub struct OpProvider;

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
        // Ensure OP token is present; allow OP_SERVICE_ACCOUNT_TOKEN or file reference.
        if let Ok(v) = std::env::var("OP_SERVICE_ACCOUNT_TOKEN") {
            if !v.is_empty() {
                return Ok(());
            }
        }
        if let Ok(path) = std::env::var("OP_SERVICE_ACCOUNT_TOKEN_FILE") {
            let mut f = std::fs::File::open(&path)
                .map_err(|e| ProviderError::Failed(format!("open token file: {}", e)))?;
            let mut buf = String::new();
            use std::io::Read as _;
            f.read_to_string(&mut buf)
                .map_err(|e| ProviderError::Failed(format!("read token file: {}", e)))?;
            let token = buf.trim_matches(|c| c == '\n' || c == '\r').to_string();
            if token.is_empty() {
                return Err(ProviderError::Failed("token file is empty".into()));
            }
            std::env::set_var("OP_SERVICE_ACCOUNT_TOKEN", &token);
            return Ok(());
        }
        Err(ProviderError::Failed(
            "OP_SERVICE_ACCOUNT_TOKEN not set (and OP_SERVICE_ACCOUNT_TOKEN_FILE not provided)"
                .into(),
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

/// Build a secrets provider from config.
pub fn build_provider(
    cfg: &Config,
) -> Result<Box<dyn SecretsProvider + Send + Sync>, ProviderError> {
    match cfg.provider.as_str() {
        "op" => Ok(Box::new(OpProvider)),
        other => Err(ProviderError::Failed(format!(
            "unsupported provider: {}",
            other
        ))),
    }
}
