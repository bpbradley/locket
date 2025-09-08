//! Secrets provider implementation
//!
//! Providers will inject secrets from templates
use anyhow::{anyhow, Context, Result};
use clap::{Args, Command as ClapCommand, FromArgMatches, Subcommand};
use secrecy::{ExposeSecret, SecretString};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use strum_macros::{AsRefStr, EnumDiscriminants, EnumString, VariantNames};

#[derive(Subcommand, Debug, Clone, EnumDiscriminants)]
#[strum_discriminants(
    name(ProviderKind),
    derive(EnumString, VariantNames, AsRefStr),
    strum(serialize_all = "lowercase")
)]
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
        let raw = std::env::var("SECRETS_PROVIDER").context("no provider configured")?;
        let kind: ProviderKind = raw.parse().map_err(|_| {
            let variants = <ProviderKind as strum::VariantNames>::VARIANTS;
            anyhow!("unsupported provider '{raw}'; supported: {:?}", variants)
        })?;

        let mut cmd = ClapCommand::new(env!("CARGO_PKG_NAME"));
        cmd = Provider::augment_subcommands(cmd);

        let m = cmd.try_get_matches_from([env!("CARGO_PKG_NAME"), kind.as_ref()])?;

        let prov = Provider::from_arg_matches(&m)?;
        Ok(prov)
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
    #[arg(
        long,
        env = "OP_SERVICE_ACCOUNT_TOKEN",
        hide_env_values = true,
        value_parser = parse_secrets
    )]
    pub token: Option<SecretString>,

    /// Path to token file (used if --token absent)
    #[arg(long, env = "OP_SERVICE_ACCOUNT_TOKEN_FILE")]
    pub token_file: Option<PathBuf>,
}

impl OpConfig {
    fn resolve_token(&self) -> Result<SecretString> {
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

fn parse_secrets(s: &str) -> Result<SecretString, String> {
    // SecretString::new expects a Box<str>
    Ok(SecretString::new(s.to_owned().into()))
}
