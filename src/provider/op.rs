//! 1password (op) based provider implementation
//! This module defines an `OpProvider` that implements
//! the `SecretsProvider` trait for fetching secrets
//!
//! It interacts with the 1Password CLI tool (`op`)
//! to retrieve secrets based on provided references.
//!
//! The provider supports authentication via service account tokens
//! and can be configured with an optional config directory.

use super::references::{HasReference, Narrow, OpReference, SecretReference};
use crate::path::AbsolutePath;
use crate::provider::config::op::OpConfig;
use crate::provider::{ConcurrencyLimit, ProviderError, SecretsProvider};
use async_trait::async_trait;
use futures::stream::{self, StreamExt};
use secrecy::ExposeSecret;
use secrecy::SecretString;
use std::collections::HashMap;
use std::process::Stdio;
use tokio::process::Command;

pub struct OpProvider {
    token: SecretString,
    config: Option<AbsolutePath>,
}

impl OpProvider {
    pub async fn new(cfg: OpConfig) -> Result<Self, ProviderError> {
        let op_token = cfg.op_token.resolve().await?;

        // Try to authenticate with the provided token
        let mut cmd = Command::new("op");
        cmd.arg("whoami")
            .env("PATH", std::env::var("PATH").unwrap_or_default())
            .env("HOME", std::env::var("HOME").unwrap_or_default())
            .env(
                "XDG_CONFIG_HOME",
                std::env::var("XDG_CONFIG_HOME").unwrap_or_default(),
            )
            .env("OP_SERVICE_ACCOUNT_TOKEN", op_token.expose_secret())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        if let Some(path) = &cfg.op_config_dir {
            cmd.env("OP_CONFIG_DIR", path.as_path());
        }

        let output = cmd.output().await.map_err(ProviderError::Io)?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(ProviderError::Unauthorized(format!(
                "op login failed: {}",
                stderr.trim()
            )));
        }

        Ok(Self {
            token: op_token,
            config: cfg.op_config_dir,
        })
    }
}

impl HasReference for OpProvider {
    type Reference = OpReference;
}

#[async_trait]
impl SecretsProvider for OpProvider {
    async fn fetch_map(
        &self,
        references: &[SecretReference],
    ) -> Result<HashMap<SecretReference, SecretString>, ProviderError> {
        const MAX_CONCURRENT_OPS: ConcurrencyLimit = ConcurrencyLimit::new(10);
        let op_refs: Vec<&OpReference> =
            references.iter().filter_map(OpReference::narrow).collect();

        if op_refs.is_empty() {
            return Ok(HashMap::new());
        }

        let results: Vec<Result<Option<(SecretReference, SecretString)>, ProviderError>> =
            stream::iter(op_refs.into_iter().cloned())
                .map(|reference| async move {
                    let key = reference.as_str();
                    let mut cmd = Command::new("op");
                    cmd.arg("read")
                        .arg("--no-newline")
                        .arg(key)
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
                        cmd.env("OP_CONFIG_DIR", path.as_path());
                    }

                    let output = cmd.output().await.map_err(ProviderError::Io)?;

                    if output.status.success() {
                        let secret = String::from_utf8(output.stdout).map_err(|e| {
                            ProviderError::InvalidConfig(format!("utf8 error: {}", e))
                        })?;

                        Ok(Some((
                            SecretReference::OnePassword(reference),
                            SecretString::new(secret.into()),
                        )))
                    } else {
                        let stderr = String::from_utf8_lossy(&output.stderr);
                        Err(ProviderError::Other(format!(
                            "op error for {}: {}",
                            key,
                            stderr.trim()
                        )))
                    }
                })
                .buffer_unordered(MAX_CONCURRENT_OPS.into_inner())
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
