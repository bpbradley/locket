use crate::provider::{AuthToken, ProviderError, SecretsProvider, macros::define_auth_token};
use async_trait::async_trait;
use clap::Args;
use futures::stream::{self, StreamExt};
use percent_encoding::percent_decode_str;
use reqwest::{Client, StatusCode};
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tracing::debug;
use url::Url;

define_auth_token!(
    struct_name: OpConnectToken,
    prefix: "connect",
    env: "OP_CONNECT_TOKEN",
    group_id: "connect_token",
    doc_string: "1Password Connect API token"
);

#[derive(Args, Debug, Clone)]
pub struct OpConnectConfig {
    /// 1Password Connect Host HTTP(S) URL
    #[arg(long = "connect.host", env = "OP_CONNECT_HOST")]
    pub host: Option<String>,

    /// 1Password Connect Token
    #[command(flatten)]
    token: OpConnectToken,

    /// Maximum allowed concurrent requests to Connect API
    #[arg(
        long = "connect.max-concurrent",
        env = "OP_CONNECT_MAX_CONCURRENT",
        default_value_t = 20
    )]
    pub connect_max_concurrent: usize,
}

impl Default for OpConnectConfig {
    fn default() -> Self {
        Self {
            host: None,
            token: OpConnectToken::default(),
            connect_max_concurrent: 20,
        }
    }
}

#[derive(Debug, Deserialize)]
struct VaultResponse {
    id: String,
}

#[derive(Debug, Deserialize)]
struct ItemResponse {
    id: String,
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

/// Cache for Name -> UUID resolution to minimize API calls
#[derive(Default, Debug)]
struct ResolutionCache {
    vaults: HashMap<String, String>,          // Vault Name -> Vault UUID
    items: HashMap<(String, String), String>, // (Vault UUID, Item Name) -> Item UUID
}

pub struct OpConnectProvider {
    client: Client,
    host: Url,
    token: AuthToken,
    cache: Arc<Mutex<ResolutionCache>>,
    max_concurrent: usize,
}

impl OpConnectProvider {
    pub async fn new(cfg: OpConnectConfig) -> Result<Self, ProviderError> {
        let token: AuthToken = cfg.token.try_into()?;
        let host_str = cfg
            .host
            .ok_or_else(|| ProviderError::InvalidConfig("missing OP_CONNECT_HOST".into()))?;

        let host = Url::parse(&host_str)
            .map_err(|e| ProviderError::InvalidConfig(format!("bad host url: {}", e)))?;

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
            .map_err(|e| ProviderError::Network(anyhow::Error::new(e)))?;

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
    async fn prewarm_cache(&self, references: &[&str]) -> Result<(), ProviderError> {
        let mut vaults = HashSet::new();
        let mut items = HashSet::new();

        for path in references {
            // Basic heuristic parse to find names/UUIDs.
            // We strip op:// and use '/' as a rough separator.
            // Strict parsing happens later in fetch_single.
            let parts: Vec<&str> = path.trim_start_matches("op://").split('/').collect();

            // We need at least Vault/Item
            if parts.len() < 2 {
                continue;
            }

            let (vault_ref, item_ref) = (parts[0], parts[1]);

            // If it's NOT a UUID, we queue it for resolution
            if !is_uuid(vault_ref) {
                vaults.insert(vault_ref.to_string());
            }
            if !is_uuid(item_ref) {
                items.insert((vault_ref.to_string(), item_ref.to_string()));
            }
        }

        stream::iter(vaults)
            .map(|vault| async move {
                let _ = self.resolve_vault_id(&vault).await;
            })
            .buffer_unordered(self.max_concurrent)
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
            .buffer_unordered(self.max_concurrent)
            .collect::<Vec<_>>()
            .await;

        Ok(())
    }

    async fn resolve_vault_id(&self, name_or_id: &str) -> Result<String, ProviderError> {
        if is_uuid(name_or_id) {
            return Ok(name_or_id.to_string());
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
        vault_uuid: &str,
        item_name_or_id: &str,
    ) -> Result<String, ProviderError> {
        if is_uuid(item_name_or_id) {
            return Ok(item_name_or_id.to_string());
        }
        let key = (vault_uuid.to_string(), item_name_or_id.to_string());
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

    async fn fetch_single(&self, raw_path: &str) -> Result<SecretString, ProviderError> {
        let url = Url::parse(raw_path)
            .map_err(|e| ProviderError::InvalidConfig(format!("bad reference URL: {}", e)))?;

        let vault_ref = url.host_str().ok_or_else(|| {
            ProviderError::InvalidConfig(format!("missing vault name: {}", raw_path))
        })?;

        let segments: Vec<&str> = url
            .path_segments()
            .ok_or_else(|| ProviderError::InvalidConfig("empty path".into()))?
            .collect();

        // Parse Item/Section/Field structure
        let (item_ref, _section, raw_field) = match segments.len() {
            2 => (segments[0], None, segments[1]),
            3 => (segments[0], Some(segments[1]), segments[2]),
            _ => {
                return Err(ProviderError::InvalidConfig(format!(
                    "invalid path segments: {}",
                    raw_path
                )));
            }
        };

        debug!(
            "Fetching secret: item: {}, section: {:?}, field: {}",
            item_ref, _section, raw_field
        );

        let field_cow = percent_decode_str(raw_field)
            .decode_utf8()
            .map_err(|e| ProviderError::InvalidConfig(format!("utf8 decode error: {}", e)))?;
        let field = field_cow.as_ref();

        let vault_id = self.resolve_vault_id(vault_ref).await?;
        let item_id = self.resolve_item_id(&vault_id, item_ref).await?;

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
            StatusCode::NOT_FOUND => return Err(ProviderError::NotFound(raw_path.to_string())),
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
            .find(|f| {
                if f.id == field {
                    return true;
                }
                if let Some(label) = &f.label
                    && label == field
                {
                    return true;
                }

                false
            })
            .ok_or_else(|| ProviderError::NotFound(format!("field '{}' not found", field)))?;

        let secret_value = target_field.value.as_ref().ok_or_else(|| {
            ProviderError::NotFound(format!("field '{}' exists but has no value", field))
        })?;

        Ok(secret_value.clone())
    }
}

#[async_trait]
impl SecretsProvider for OpConnectProvider {
    fn accepts_key(&self, key: &str) -> bool {
        key.starts_with("op://")
    }

    async fn fetch_map(
        &self,
        references: &[&str],
    ) -> Result<HashMap<String, SecretString>, ProviderError> {
        // We must first resolve any vault or item names to UUIDs.
        // So we first collect all unique names, and pre-resolve them
        // into cache so that we don't need to resolve these again in the future
        if let Err(e) = self.prewarm_cache(references).await {
            tracing::warn!("cache pre-warm failed: {}", e);
        }

        let refs: Vec<String> = references.iter().map(|s| s.to_string()).collect();

        let results: Vec<Result<Option<(String, SecretString)>, ProviderError>> =
            stream::iter(refs)
                .map(|key| async move {
                    match self.fetch_single(&key).await {
                        Ok(val) => Ok(Some((key, val))),
                        Err(e) => Err(e),
                    }
                })
                .buffer_unordered(self.max_concurrent)
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

fn is_uuid(s: &str) -> bool {
    s.len() == 26
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
}
