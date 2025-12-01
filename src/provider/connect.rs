use crate::provider::{AuthToken, ProviderError, SecretsProvider};
use async_trait::async_trait;
use clap::Args;
use futures::stream::{self, StreamExt};
use percent_encoding::percent_decode_str;
use reqwest::{Client, StatusCode};
use secrecy::{ExposeSecret, SecretString};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tracing::debug;
use url::Url;

#[derive(Args, Debug, Clone, Default)]
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
    pub max_concurrent: usize,
}

#[derive(Args, Debug, Clone, Default)]
#[group(id = "connect_token", multiple = false, required = true)]
pub struct OpConnectToken {
    /// 1Password Connect API token
    #[arg(
        long = "connect.token",
        env = "OP_CONNECT_TOKEN",
        hide_env_values = true
    )]
    connect_val: Option<SecretString>,

    /// Path to file containing 1Password Connect API token
    #[arg(long = "connect.token-file", env = "OP_CONNECT_TOKEN_FILE")]
    connect_file: Option<PathBuf>,
}

impl OpConnectToken {
    pub fn resolve(&self) -> Result<AuthToken, ProviderError> {
        AuthToken::try_new(
            self.connect_val.clone(),
            self.connect_file.clone(),
            "OpConnect",
        )
    }
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
    pub fn new(cfg: OpConnectConfig) -> Result<Self, ProviderError> {
        let host_str = cfg
            .host
            .ok_or_else(|| ProviderError::InvalidConfig("missing OP_CONNECT_HOST".into()))?;

        let token = cfg.token.resolve()?;

        let host = Url::parse(&host_str)
            .map_err(|e| ProviderError::InvalidConfig(format!("bad host url: {}", e)))?;

        let client = Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .map_err(|e| ProviderError::Other(e.to_string()))?;

        Ok(Self {
            client,
            host,
            token,
            cache: Arc::new(Mutex::new(ResolutionCache::default())),
            max_concurrent: cfg.max_concurrent,
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

        // Resolve Vaults
        stream::iter(vaults)
            .map(|vault| async move {
                let _ = self.resolve_vault_id(&vault).await;
            })
            .buffer_unordered(self.max_concurrent)
            .collect::<Vec<_>>()
            .await;

        // Resolve Items
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

        let vaults: Vec<Value> = resp
            .json()
            .await
            .map_err(|e| ProviderError::Network(e.into()))?;

        let vault = vaults
            .first()
            .ok_or_else(|| ProviderError::NotFound(format!("vault '{}' not found", name_or_id)))?;

        let uuid = vault["id"]
            .as_str()
            .ok_or_else(|| ProviderError::Other("vault response missing id".into()))?
            .to_string();

        {
            let mut cache = self.cache.lock().await;
            cache.vaults.insert(name_or_id.to_string(), uuid.clone());
        }
        Ok(uuid)
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

        let items: Vec<Value> = resp
            .json()
            .await
            .map_err(|e| ProviderError::Network(e.into()))?;

        let item = items.first().ok_or_else(|| {
            ProviderError::NotFound(format!("item '{}' not found in vault", item_name_or_id))
        })?;

        let uuid = item["id"]
            .as_str()
            .ok_or_else(|| ProviderError::Other("item response missing id".into()))?
            .to_string();

        {
            let mut cache = self.cache.lock().await;
            cache.items.insert(key, uuid.clone());
        }

        Ok(uuid)
    }

    async fn fetch_single(&self, raw_path: &str) -> Result<String, ProviderError> {
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

        // Resolve UUIDs
        let vault_id = self.resolve_vault_id(vault_ref).await?;
        let item_id = self.resolve_item_id(&vault_id, item_ref).await?;

        // Fetch Item
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

        let item_json: Value = resp
            .json()
            .await
            .map_err(|e| ProviderError::Network(e.into()))?;

        // Find Field Value
        let field_obj = item_json["fields"]
            .as_array()
            .and_then(|fields| {
                fields.iter().find(|f| {
                    let f_id = f["id"].as_str().unwrap_or("");
                    let f_label = f["label"].as_str().unwrap_or("");

                    // Strict match against ID or Label (using the decoded string)
                    f_id == field || f_label == field
                })
            })
            .ok_or_else(|| ProviderError::NotFound(format!("field '{}' not found", field)))?;

        let raw_value = field_obj["value"]
            .as_str()
            .ok_or_else(|| ProviderError::NotFound("field has no value".into()))?;

        Ok(raw_value.to_string())
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
    ) -> Result<HashMap<String, String>, ProviderError> {
        // We must first resolve any vault or item names to UUIDs.
        // So we first collect all unique names, and pre-resolve them
        // into cache so that we don't need to resolve these again in the future
        if let Err(e) = self.prewarm_cache(references).await {
            tracing::warn!("cache pre-warm failed: {}", e);
        }

        let refs: Vec<String> = references.iter().map(|s| s.to_string()).collect();

        let results: Vec<Result<Option<(String, String)>, ProviderError>> = stream::iter(refs)
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
