//! 1Password Connect provider implementation.
//! This module defines an `OpConnectProvider` that implements
//! the `SecretsProvider` trait for fetching secrets from
//! a 1Password Connect server.
//!
//! It supports resolving vault and item names to UUIDs,
//! fetching item details, and extracting secret fields.
//! It also includes caching for name-to-UUID resolution
//! to minimize API calls.
//!
//! The provider uses the reqwest HTTP client
//! and handles authentication via bearer tokens.

use super::references::{OpReference, ReferenceParser, SecretReference};
use crate::provider::ConcurrencyLimit;
use crate::provider::{AuthToken, ProviderError, SecretsProvider};
use async_trait::async_trait;
use clap::Args;
use futures::stream::{self, StreamExt};
use reqwest::{Client, StatusCode};
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use url::Url;

#[derive(Args, Debug, Clone)]
pub struct OpConnectConfig {
    /// 1Password Connect Host HTTP(S) URL
    #[arg(long = "connect.host", env = "OP_CONNECT_HOST")]
    host: Option<Url>,

    /// 1Password Connect Token
    /// Either provide the token directly or via a file with `file:` prefix
    #[arg(
        long = "connect.token",
        env = "OP_SERVICE_ACCOUNT_TOKEN",
        hide_env_values = true
    )]
    connect_token: Option<AuthToken>,

    /// Maximum allowed concurrent requests to Connect API
    #[arg(
        long = "connect.max-concurrent",
        env = "OP_CONNECT_MAX_CONCURRENT",
        default_value_t = ConcurrencyLimit::new(20)
    )]
    connect_max_concurrent: ConcurrencyLimit,
}

#[derive(Debug, Deserialize)]
struct VaultResponse {
    id: VaultId,
}

#[derive(Debug, Deserialize)]
struct ItemResponse {
    id: ItemId,
}

#[derive(Debug, Deserialize)]
struct ConnectItemDetail {
    fields: Option<Vec<ConnectField>>,
}

#[derive(Debug, Deserialize)]
struct ConnectField {
    id: String,
    label: Option<String>,
    value: Option<SecretString>,
}

#[derive(Debug, Deserialize)]
struct ErrorResponse {
    message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize)]
#[serde(transparent)]
struct OpUuid(String);

impl fmt::Display for OpUuid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for OpUuid {
    type Err = ProviderError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.len() == 26
            && s.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        {
            Ok(Self(s.to_string()))
        } else {
            Err(ProviderError::InvalidId(format!("invalid id '{}'", s)))
        }
    }
}

impl AsRef<str> for OpUuid {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize)]
#[serde(transparent)]
struct VaultId(OpUuid);

impl fmt::Display for VaultId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl FromStr for VaultId {
    type Err = ProviderError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        OpUuid::from_str(s)
            .map(Self)
            .map_err(|_| ProviderError::InvalidId(format!("invalid vault id '{}'", s)))
    }
}

impl AsRef<str> for VaultId {
    fn as_ref(&self) -> &str {
        self.0.as_ref()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize)]
#[serde(transparent)]
struct ItemId(OpUuid);

impl fmt::Display for ItemId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl FromStr for ItemId {
    type Err = ProviderError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        OpUuid::from_str(s)
            .map(Self)
            .map_err(|_| ProviderError::InvalidId(format!("invalid item id '{}'", s)))
    }
}

impl AsRef<str> for ItemId {
    fn as_ref(&self) -> &str {
        self.0.as_ref()
    }
}

/// Cache for Name -> UUID resolution to minimize API calls
#[derive(Default, Debug)]
struct ResolutionCache {
    vaults: HashMap<String, VaultId>,
    items: HashMap<(VaultId, String), ItemId>,
}

pub struct OpConnectProvider {
    client: Client,
    host: Url,
    token: AuthToken,
    cache: Arc<Mutex<ResolutionCache>>,
    max_concurrent: ConcurrencyLimit,
}

#[cfg(any(feature = "op", feature = "connect"))]
impl ReferenceParser for OpConnectProvider {
    fn parse(&self, raw: &str) -> Option<SecretReference> {
        OpReference::from_str(raw)
            .ok()
            .map(SecretReference::OnePassword)
    }
}

impl OpConnectProvider {
    pub async fn new(cfg: OpConnectConfig) -> Result<Self, ProviderError> {
        let token = cfg.connect_token.ok_or_else(|| {
            ProviderError::InvalidConfig(
                "missing 1Password service account token (connect.token)".to_string(),
            )
        })?;

        let host = cfg
            .host
            .ok_or_else(|| ProviderError::InvalidConfig("missing OP_CONNECT_HOST".into()))?;

        let client = Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .map_err(|e| ProviderError::Other(e.to_string()))?;

        let check_url = host
            .join("/v1/vaults")
            .map_err(|e| ProviderError::Other(e.to_string()))?;

        let resp = client
            .get(check_url)
            .bearer_auth(token.expose_secret())
            .send()
            .await
            .map_err(|e| ProviderError::Network(Box::new(e)))?;

        let status = resp.status();

        if !status.is_success() {
            let error_msg = resp
                .json::<ErrorResponse>()
                .await
                .ok()
                .and_then(|e| e.message)
                .unwrap_or_else(|| status.to_string());

            return match status {
                StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
                    Err(ProviderError::Unauthorized(error_msg))
                }
                _ => Err(ProviderError::Other(format!(
                    "connect error: {}",
                    error_msg
                ))),
            };
        }

        Ok(Self {
            client,
            host,
            token,
            cache: Arc::new(Mutex::new(ResolutionCache::default())),
            max_concurrent: cfg.connect_max_concurrent,
        })
    }

    /// Pre-resolves all Vault and Item names found in the reference list.
    async fn prewarm_cache(&self, references: &[&OpReference]) -> Result<(), ProviderError> {
        let mut vaults = HashSet::new();
        let mut items = HashSet::new();

        for reference in references {
            if reference.vault.parse::<VaultId>().is_err() {
                vaults.insert(reference.vault.clone());
            }

            if reference.item.parse::<ItemId>().is_err() {
                items.insert((reference.vault.clone(), reference.item.clone()));
            }
        }

        stream::iter(vaults)
            .map(|vault| async move {
                let _ = self.resolve_vault_id(&vault).await;
            })
            .buffer_unordered(self.max_concurrent.into_inner())
            .collect::<Vec<_>>()
            .await;

        stream::iter(items)
            .map(|(vault, item)| async move {
                let vault_uuid = match self.resolve_vault_id(&vault).await {
                    Ok(id) => id,
                    Err(_) => return, // Skip item if vault failed
                };
                let _ = self.resolve_item_id(&vault_uuid, &item).await;
            })
            .buffer_unordered(self.max_concurrent.into_inner())
            .collect::<Vec<_>>()
            .await;

        Ok(())
    }

    async fn resolve_vault_id(&self, name_or_id: &str) -> Result<VaultId, ProviderError> {
        if let Ok(id) = name_or_id.parse::<VaultId>() {
            return Ok(id);
        }

        {
            let cache = self.cache.lock().await;
            if let Some(uuid) = cache.vaults.get(name_or_id) {
                return Ok(uuid.clone());
            }
        }

        let url = self
            .host
            .join("/v1/vaults")
            .map_err(|e| ProviderError::Other(e.to_string()))?;

        let filter = format!("name eq \"{}\"", name_or_id);

        let resp = self
            .client
            .get(url)
            .query(&[("filter", &filter)])
            .bearer_auth(self.token.expose_secret())
            .send()
            .await
            .map_err(|e| ProviderError::Network(e.into()))?;

        if !resp.status().is_success() {
            return Err(ProviderError::Other(format!(
                "vault lookup failed: {}",
                resp.status()
            )));
        }

        let vaults: Vec<VaultResponse> = resp
            .json()
            .await
            .map_err(|e| ProviderError::Network(e.into()))?;

        let vault = vaults
            .first()
            .ok_or_else(|| ProviderError::NotFound(format!("vault '{}' not found", name_or_id)))?;

        {
            let mut cache = self.cache.lock().await;
            cache
                .vaults
                .insert(name_or_id.to_string(), vault.id.clone());
        }
        Ok(vault.id.clone())
    }

    async fn resolve_item_id(
        &self,
        vault_uuid: &VaultId,
        item_name_or_id: &str,
    ) -> Result<ItemId, ProviderError> {
        if let Ok(id) = item_name_or_id.parse::<ItemId>() {
            return Ok(id);
        }
        let key = (vault_uuid.clone(), item_name_or_id.to_string());
        {
            let cache = self.cache.lock().await;
            if let Some(uuid) = cache.items.get(&key) {
                return Ok(uuid.clone());
            }
        }

        let path = format!("/v1/vaults/{}/items", vault_uuid);
        let url = self
            .host
            .join(&path)
            .map_err(|e| ProviderError::Other(e.to_string()))?;

        let filter = format!("title eq \"{}\"", item_name_or_id);

        let resp = self
            .client
            .get(url)
            .query(&[("filter", &filter)])
            .bearer_auth(self.token.expose_secret())
            .send()
            .await
            .map_err(|e| ProviderError::Network(e.into()))?;

        let items: Vec<ItemResponse> = resp
            .json()
            .await
            .map_err(|e| ProviderError::Network(e.into()))?;

        let item = items.first().ok_or_else(|| {
            ProviderError::NotFound(format!("item '{}' not found in vault", item_name_or_id))
        })?;

        {
            let mut cache = self.cache.lock().await;
            cache.items.insert(key, item.id.clone());
        }

        Ok(item.id.clone())
    }

    async fn fetch_single(&self, op_ref: &OpReference) -> Result<SecretString, ProviderError> {
        let vault_id = self.resolve_vault_id(&op_ref.vault).await?;
        let item_id = self.resolve_item_id(&vault_id, &op_ref.item).await?;

        let mut api_url = self.host.clone();
        api_url.set_path(&format!("/v1/vaults/{}/items/{}", vault_id, item_id));

        let resp = self
            .client
            .get(api_url)
            .bearer_auth(self.token.expose_secret())
            .send()
            .await
            .map_err(|e| ProviderError::Network(e.into()))?;

        match resp.status() {
            StatusCode::OK => {}
            StatusCode::NOT_FOUND => {
                return Err(ProviderError::NotFound(op_ref.to_string()));
            }
            StatusCode::UNAUTHORIZED => {
                return Err(ProviderError::Unauthorized("invalid token".into()));
            }
            s => return Err(ProviderError::Other(format!("connect api error: {}", s))),
        }

        let item_detail: ConnectItemDetail = resp
            .json()
            .await
            .map_err(|e| ProviderError::Network(e.into()))?;

        let fields = item_detail.fields.as_deref().unwrap_or(&[]);

        let target_field = fields
            .iter()
            .find(|f| f.id == op_ref.field || f.label.as_ref() == Some(&op_ref.field))
            .ok_or_else(|| {
                ProviderError::NotFound(format!("field '{}' not found", op_ref.field))
            })?;

        let secret_value = target_field.value.as_ref().ok_or_else(|| {
            ProviderError::NotFound(format!("field '{}' exists but has no value", op_ref.field))
        })?;

        Ok(secret_value.clone())
    }
}

#[async_trait]
impl SecretsProvider for OpConnectProvider {
    async fn fetch_map(
        &self,
        references: &[SecretReference],
    ) -> Result<HashMap<SecretReference, SecretString>, ProviderError> {
        let op_refs: Vec<&OpReference> = references.iter().filter_map(|r| r.as_op()).collect();

        if op_refs.is_empty() {
            return Ok(HashMap::new());
        }

        // We must first resolve any vault or item names to UUIDs.
        // So we first collect all unique names, and pre-resolve them
        // into cache so that we don't need to resolve these again in the future
        if let Err(e) = self.prewarm_cache(&op_refs).await {
            tracing::warn!("cache pre-warm failed: {}", e);
        }

        let results: Vec<Result<Option<(SecretReference, SecretString)>, ProviderError>> =
            stream::iter(op_refs.into_iter().cloned())
                .map(|op_ref| async move {
                    match self.fetch_single(&op_ref).await {
                        Ok(val) => Ok(Some((SecretReference::OnePassword(op_ref), val))),
                        Err(e) => Err(e),
                    }
                })
                .buffer_unordered(self.max_concurrent.into_inner())
                .collect::<Vec<_>>()
                .await;

        // Aggregate
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
