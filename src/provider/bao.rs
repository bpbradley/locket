//! OpenBao / HashiCorp Vault provider implementation.
//!
//! Uses the KV v2 secrets engine to fetch secrets, and AppRole auth
//! for authentication.
//!
//! The authentication token is lazily refreshed when it expires
//! and it will gracefully handle rotating authentication when access is denied.

use super::{
    ConcurrencyLimit, ProviderError, SecretsProvider,
    config::bao::BaoConfig,
    references::{BaoReference, BaoSecretLocation, Extract, HasReference, SecretReference},
};
use async_trait::async_trait;
use futures::{StreamExt, stream};
use reqwest::{Client, StatusCode};
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tracing::{debug, warn};
use url::Url;

pub struct BaoProvider {
    client: Client,
    config: ProviderConfig,
    auth: BaoAuthenticator,
}

impl BaoProvider {
    pub async fn new(config: BaoConfig) -> Result<Self, ProviderError> {
        let secret_id = config.bao_secret_id.resolve().await?;
        let auth_config = AuthConfig {
            url: config.bao_url.clone(),
            namespace: config.bao_namespace.clone(),
            auth_mount: config.bao_auth_mount.clone(),
            role_id: config.bao_role_id.clone(),
            secret_id,
        };

        let provider_config = ProviderConfig::from(config);

        let client = Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .map_err(|e| ProviderError::Other(e.to_string()))?;

        let auth = BaoAuthenticator::try_new(client.clone(), auth_config).await?;

        Ok(Self {
            client,
            config: provider_config,
            auth,
        })
    }

    /// Reads a KV v2 secret's full data map for a given location.
    async fn fetch_group(
        &self,
        location: &BaoSecretLocation,
        token: &SecretString,
    ) -> Result<HashMap<String, serde_json::Value>, ProviderError> {
        let mut url = self.config.url.clone();
        {
            let mut segments = url
                .path_segments_mut()
                .map_err(|_| ProviderError::InvalidConfig("bao-url cannot be a base".into()))?;
            segments.clear();
            segments.push("v1");
            segments.push(location.mount.as_str());
            segments.push("data");
            segments.extend(location.path.segments());
        }

        let mut req = self
            .client
            .get(url)
            .header("X-Vault-Token", token.expose_secret());
        if let Some(ns) = &self.config.namespace {
            req = req.header("X-Vault-Namespace", ns);
        }

        let resp = req
            .send()
            .await
            .map_err(|e| ProviderError::Network(Box::new(e)))?;

        match resp.status() {
            StatusCode::OK => {
                let wrapper: KvV2Response = resp
                    .json()
                    .await
                    .map_err(|e| ProviderError::Network(Box::new(e)))?;
                wrapper
                    .data
                    .data
                    .ok_or_else(|| ProviderError::NotFound(location.to_string()))
            }
            StatusCode::NOT_FOUND => Err(ProviderError::NotFound(location.to_string())),
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => Err(ProviderError::Unauthorized(
                format!("Access denied for {}", location),
            )),
            status => {
                let txt = resp.text().await.unwrap_or_default();
                Err(ProviderError::Other(format!(
                    "OpenBao error {}: {}",
                    status, txt
                )))
            }
        }
    }

    /// Fetches a secret's data map, retrying once with a fresh token if access was denied.
    async fn fetch_group_with_retry(
        &self,
        location: &BaoSecretLocation,
    ) -> Result<HashMap<String, serde_json::Value>, ProviderError> {
        let mut attempt = 0;
        loop {
            attempt += 1;
            let token = self.auth.get_token().await?;

            match self.fetch_group(location, &token).await {
                Ok(data) => return Ok(data),
                // Token may need to be refreshed. Try invalidating the token
                // to trigger a rotation and try again
                Err(ProviderError::Unauthorized(_)) if attempt < 2 => {
                    warn!(
                        "Got Unauthorized for {}. Invalidating token and retrying...",
                        location
                    );
                    self.auth.invalidate(&token).await;
                    continue;
                }
                Err(e) => return Err(e),
            }
        }
    }
}

impl HasReference for BaoProvider {
    type Reference = BaoReference;
}

#[async_trait]
impl SecretsProvider for BaoProvider {
    async fn fetch_map(
        &self,
        references: &[SecretReference],
    ) -> Result<HashMap<SecretReference, SecretString>, ProviderError> {
        // Group references by location so a secret with multiple referenced
        // fields is only fetched once, instead of once per field.
        let mut groups: HashMap<&BaoSecretLocation, Vec<&BaoReference>> = HashMap::new();
        for r in references.iter().filter_map(BaoReference::extract) {
            groups.entry(&r.location).or_default().push(r);
        }

        if groups.is_empty() {
            return Ok(HashMap::new());
        }

        let fetches: Vec<_> = groups
            .into_iter()
            .map(|(location, group_refs)| async move {
                let data = self.fetch_group_with_retry(location).await;
                (group_refs, data)
            })
            .collect();

        let results = stream::iter(fetches)
            .buffer_unordered(self.config.max_concurrent.into_inner())
            .collect::<Vec<_>>()
            .await;

        let mut map = HashMap::new();
        for (group_refs, data) in results {
            match data {
                Ok(fields) => {
                    for r in group_refs {
                        match fields.get(r.field.as_str()) {
                            Some(serde_json::Value::String(s)) => {
                                let secret = SecretString::new(s.clone().into());
                                map.insert(SecretReference::Bao(r.clone()), secret);
                            }
                            Some(_) => {
                                warn!(
                                    "Field '{}' in {} is not a string; skipping",
                                    r.field, r.location
                                );
                            }
                            None => {
                                // Field not present in the secret's data map.
                                // Leave unresolved, per fetch_map contract.
                            }
                        }
                    }
                }
                // Whole secret not found: leave all of its fields unresolved.
                Err(ProviderError::NotFound(_)) => {}
                Err(e) => return Err(e),
            }
        }

        Ok(map)
    }
}

/// Handles authentication and token renewal for OpenBao / Vault
///
/// Tokens are automatically renewed when they expire or when
/// intentionally invalidated.
///
/// Uses the AppRole auth method for authentication, and the token
/// is held in a RwLock to allow concurrent reads and exclusive writes
struct BaoAuthenticator {
    client: Client,
    config: AuthConfig,
    token: RwLock<BaoToken>,
}

impl BaoAuthenticator {
    pub async fn try_new(client: Client, config: AuthConfig) -> Result<Self, ProviderError> {
        let token = Self::login(&client, &config).await?;

        Ok(Self {
            client,
            config,
            token: RwLock::new(token),
        })
    }

    /// Returns a valid client token, renewing it if necessary.
    pub async fn get_token(&self) -> Result<SecretString, ProviderError> {
        {
            let guard = self.token.read().await;
            if !guard.is_expired() {
                return Ok(guard.client_token.clone());
            }
        }

        // Token expired. Need to renew
        let mut guard = self.token.write().await;

        // Check if token is expired again in case it was renewed by another thread
        // while waiting for the write lock
        if !guard.is_expired() {
            return Ok(guard.client_token.clone());
        }

        debug!("Token expired. Renewing...");
        let new_token = Self::login(&self.client, &self.config).await?;

        *guard = new_token.clone();

        Ok(new_token.client_token)
    }

    async fn login(client: &Client, config: &AuthConfig) -> Result<BaoToken, ProviderError> {
        let mut url = config.url.clone();
        {
            let mut segments = url
                .path_segments_mut()
                .map_err(|_| ProviderError::InvalidConfig("bao-url cannot be a base".into()))?;
            segments.clear();
            segments.push("v1");
            segments.push("auth");
            segments.push(&config.auth_mount);
            segments.push("login");
        }

        let payload = LoginParams {
            role_id: &config.role_id,
            secret_id: SecretIdView(&config.secret_id),
        };

        let mut req = client.post(url).json(&payload);
        if let Some(ns) = &config.namespace {
            req = req.header("X-Vault-Namespace", ns);
        }

        let resp = req
            .send()
            .await
            .map_err(|e| ProviderError::Network(Box::new(e)))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(ProviderError::Unauthorized(format!(
                "AppRole login failed: {} - {}",
                status, text
            )));
        }

        let login_resp: LoginResponse = resp
            .json()
            .await
            .map_err(|e| ProviderError::Network(Box::new(e)))?;

        debug!(
            "Login successful. Expires in {} seconds",
            login_resp.auth.lease_duration
        );

        Ok(BaoToken {
            client_token: login_resp.auth.client_token,
            expiry: Instant::now() + Duration::from_secs(login_resp.auth.lease_duration),
        })
    }

    async fn invalidate(&self, token: &SecretString) {
        let mut guard = self.token.write().await;
        if guard.client_token.expose_secret() == token.expose_secret() {
            guard.poison();
        }
    }
}

#[derive(Debug, Clone)]
struct AuthConfig {
    url: Url,
    namespace: Option<String>,
    auth_mount: String,
    role_id: String,
    secret_id: SecretString,
}

#[derive(Debug, Clone)]
struct ProviderConfig {
    url: Url,
    namespace: Option<String>,
    max_concurrent: ConcurrencyLimit,
}

impl From<BaoConfig> for ProviderConfig {
    fn from(config: BaoConfig) -> Self {
        ProviderConfig {
            url: config.bao_url,
            namespace: config.bao_namespace,
            max_concurrent: config.bao_max_concurrent,
        }
    }
}

#[derive(Debug, Clone)]
struct BaoToken {
    client_token: SecretString,
    expiry: Instant,
}

impl BaoToken {
    fn is_expired(&self) -> bool {
        self.expiry <= Instant::now() + Duration::from_secs(60)
    }
    fn poison(&mut self) {
        // Set to a point in the past so that it will be considered expired
        self.expiry = Instant::now() - Duration::from_secs(1);
    }
}

struct SecretIdView<'a>(&'a SecretString);

impl<'a> Serialize for SecretIdView<'a> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(self.0.expose_secret())
    }
}

#[derive(Serialize)]
struct LoginParams<'a> {
    role_id: &'a str,
    secret_id: SecretIdView<'a>,
}

#[derive(Deserialize)]
struct LoginResponse {
    auth: LoginAuth,
}

#[derive(Deserialize)]
struct LoginAuth {
    client_token: SecretString,
    lease_duration: u64,
}

#[derive(Deserialize)]
struct KvV2Response {
    data: KvV2Data,
}

#[derive(Deserialize)]
struct KvV2Data {
    data: Option<HashMap<String, serde_json::Value>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn token_with_expiry(expiry: Instant) -> BaoToken {
        BaoToken {
            client_token: SecretString::new("test-token".into()),
            expiry,
        }
    }

    #[test]
    fn test_token_not_expired_well_before_leeway() {
        let token = token_with_expiry(Instant::now() + Duration::from_secs(120));
        assert!(!token.is_expired());
    }

    #[test]
    fn test_token_expired_within_leeway() {
        // is_expired() treats tokens expiring within the next 60s as expired already,
        // so renewal happens before the token actually stops working.
        let token = token_with_expiry(Instant::now() + Duration::from_secs(30));
        assert!(token.is_expired());
    }

    #[test]
    fn test_token_expired_in_the_past() {
        let token = token_with_expiry(Instant::now() - Duration::from_secs(1));
        assert!(token.is_expired());
    }

    #[test]
    fn test_token_poison_marks_expired() {
        let mut token = token_with_expiry(Instant::now() + Duration::from_secs(120));
        assert!(!token.is_expired());
        token.poison();
        assert!(token.is_expired());
    }
}
