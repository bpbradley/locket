//! 1Password (op) provider backed by the `locket-op-bridge` sidecar.
//!
//! All process and protocol machinery lives in [`bridge`]; this module
//! is only the [`SecretsProvider`] glue: reference filtering, batch
//! resolution through the bridge, and stitching results back to keys.
//!
//! Authentication is via service account token, sent once over the
//! bridge's private pipe at startup (never argv or env).

mod bridge;

use super::references::{Extract, HasReference, OpReference, SecretReference};
use crate::provider::config::op::OpConfig;
use crate::provider::{ProviderError, SecretsProvider};
use async_trait::async_trait;
use bridge::{Bridge, ResolveResult};
use secrecy::SecretString;
use std::collections::HashMap;

pub struct OpProvider {
    bridge: Bridge,
}

impl OpProvider {
    pub async fn new(cfg: OpConfig) -> Result<Self, ProviderError> {
        let token = cfg.op_token.resolve().await?;
        let bridge = Bridge::connect(cfg.op_bridge.as_ref(), &token).await?;
        Ok(Self { bridge })
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
        let op_refs: Vec<&OpReference> =
            references.iter().filter_map(OpReference::extract).collect();

        if op_refs.is_empty() {
            return Ok(HashMap::new());
        }

        let refs: Vec<&str> = op_refs.iter().map(|r| r.as_str()).collect();
        let mut results = self.bridge.resolve(&refs).await?;

        let mut map = HashMap::with_capacity(op_refs.len());
        for reference in op_refs {
            match results.remove(reference.as_str()) {
                Some(ResolveResult::Resolved { secret }) => {
                    map.insert(SecretReference::OnePassword(reference.clone()), secret);
                }
                Some(ResolveResult::Failed { error }) => {
                    return Err(error.code.into_provider_error(format!(
                        "{}: {}",
                        reference.as_str(),
                        error.message
                    )));
                }
                None => {
                    return Err(ProviderError::Other(format!(
                        "op bridge response missing reference {}",
                        reference.as_str()
                    )));
                }
            }
        }

        Ok(map)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use secrecy::ExposeSecret;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    /// Provider over a scripted bridge that answers every resolve
    /// request from a fixed results payload.
    fn scripted_provider(results_json: &'static str) -> OpProvider {
        let (locket_end, bridge_end) = tokio::io::duplex(64 * 1024);
        let (l_read, l_write) = tokio::io::split(locket_end);
        let (b_read, mut b_write) = tokio::io::split(bridge_end);
        tokio::spawn(async move {
            let mut lines = BufReader::new(b_read).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let request: serde_json::Value = serde_json::from_str(&line).unwrap();
                let id = request["id"].as_u64().unwrap();
                let response =
                    format!(r#"{{"type":"resolve-ok","id":{id},"results":{results_json}}}"#);
                b_write
                    .write_all(format!("{response}\n").as_bytes())
                    .await
                    .unwrap();
            }
        });
        OpProvider {
            bridge: Bridge::from_pipes(l_read, l_write),
        }
    }

    fn op_ref(raw: &str) -> SecretReference {
        SecretReference::OnePassword(raw.parse().unwrap())
    }

    #[tokio::test]
    async fn fetch_map_ignores_non_op_references() {
        let provider = scripted_provider("{}");
        let refs = [SecretReference::Mock("not-op".into())];
        let map = provider.fetch_map(&refs).await.unwrap();
        assert!(map.is_empty());
    }

    #[tokio::test]
    async fn fetch_map_stitches_results_to_references() {
        let provider = scripted_provider(r#"{"op://v/i/f":{"secret":"hunter2"}}"#);
        let reference = op_ref("op://v/i/f");
        let map = provider
            .fetch_map(std::slice::from_ref(&reference))
            .await
            .unwrap();
        assert_eq!(map[&reference].expose_secret(), "hunter2");
    }

    #[tokio::test]
    async fn fetch_map_fails_on_per_reference_error() {
        let provider = scripted_provider(
            r#"{"op://v/i/f":{"error":{"code":"not_found","message":"itemNotFound"}}}"#,
        );
        let refs = [op_ref("op://v/i/f")];
        let err = provider.fetch_map(&refs).await.unwrap_err();
        assert!(
            matches!(&err, ProviderError::NotFound(m) if m.contains("op://v/i/f")),
            "{err}"
        );
    }

    #[tokio::test]
    async fn fetch_map_fails_on_missing_reference() {
        let provider = scripted_provider("{}");
        let refs = [op_ref("op://v/i/f")];
        let err = provider.fetch_map(&refs).await.unwrap_err();
        assert!(
            matches!(&err, ProviderError::Other(m) if m.contains("missing reference")),
            "{err}"
        );
    }
}
