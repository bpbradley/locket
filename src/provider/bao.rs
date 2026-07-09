//! OpenBao / HashiCorp Vault provider implementation.
//!
//! Uses the KV v2 secrets engine to fetch secrets, and AppRole auth
//! for authentication.
//!
//! The authentication token is lazily refreshed when it expires
//! and it will gracefully handle rotating authentication when access is denied.

use super::{
    ConcurrencyLimit, ProviderError, SecretsProvider, ServerUrl,
    auth::{ExpiringToken, SecretView, TokenAuthenticator, TokenExchange},
    config::bao::{BaoConfig, BaoNamespace},
    references::{
        BaoMount, BaoReference, BaoSecretLocation, Extract, HasReference, SecretReference,
    },
};
use async_trait::async_trait;
use futures::{StreamExt, stream};
use reqwest::{Client, StatusCode};
use secrecy::{ExposeSecret, SecretString};
use serde::de::{self, IgnoredAny, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;
use std::time::Duration;
use tracing::warn;

pub struct BaoProvider {
    client: Client,
    config: ProviderConfig,
    auth: TokenAuthenticator<AppRoleLogin>,
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

        let auth = TokenAuthenticator::try_new(AppRoleLogin {
            client: client.clone(),
            config: auth_config,
        })
        .await?;

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
    ) -> Result<HashMap<String, KvV2Value>, ProviderError> {
        let url = self.config.url.endpoint(
            ["v1", location.mount.as_str(), "data"]
                .into_iter()
                .chain(location.path.segments()),
        );

        let mut req = self
            .client
            .get(url)
            .header("X-Vault-Token", token.expose_secret());
        if let Some(ns) = &self.config.namespace {
            req = req.header("X-Vault-Namespace", ns.as_str());
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
    ) -> Result<HashMap<String, KvV2Value>, ProviderError> {
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
                            Some(KvV2Value::Scalar(secret)) => {
                                map.insert(SecretReference::Bao(r.clone()), secret.clone());
                            }
                            Some(KvV2Value::Unsupported) => {
                                warn!(
                                    "Field '{}' in {} is not a scalar value; skipping",
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

/// AppRole credential exchange for OpenBao / Vault.
struct AppRoleLogin {
    client: Client,
    config: AuthConfig,
}

#[async_trait]
impl TokenExchange for AppRoleLogin {
    async fn login(&self) -> Result<ExpiringToken, ProviderError> {
        let url =
            self.config
                .url
                .endpoint(["v1", "auth", self.config.auth_mount.as_str(), "login"]);

        let payload = LoginParams {
            role_id: &self.config.role_id,
            secret_id: SecretView(&self.config.secret_id),
        };

        let mut req = self.client.post(url).json(&payload);
        if let Some(ns) = &self.config.namespace {
            req = req.header("X-Vault-Namespace", ns.as_str());
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

        Ok(ExpiringToken::new(
            login_resp.auth.client_token,
            login_resp.auth.lease_duration,
        ))
    }
}

#[derive(Debug, Clone)]
struct AuthConfig {
    url: ServerUrl,
    namespace: Option<BaoNamespace>,
    auth_mount: BaoMount,
    role_id: String,
    secret_id: SecretString,
}

#[derive(Debug, Clone)]
struct ProviderConfig {
    url: ServerUrl,
    namespace: Option<BaoNamespace>,
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

#[derive(Serialize)]
struct LoginParams<'a> {
    role_id: &'a str,
    secret_id: SecretView<'a>,
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
    data: Option<HashMap<String, KvV2Value>>,
}

/// A single field value in a KV v2 secret's data map.
///
/// Scalars are captured as secrets at deserialization time so plaintext
/// never sits in a non-zeroizing type. Numbers and bools resolve to their
/// string form. Nulls, arrays, and objects cannot be injected as a value.
enum KvV2Value {
    Scalar(SecretString),
    Unsupported,
}

impl<'de> Deserialize<'de> for KvV2Value {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct KvV2ValueVisitor;

        impl<'de> Visitor<'de> for KvV2ValueVisitor {
            type Value = KvV2Value;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a KV v2 field value")
            }

            fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
                Ok(KvV2Value::Scalar(SecretString::new(v.into())))
            }

            fn visit_string<E: de::Error>(self, v: String) -> Result<Self::Value, E> {
                Ok(KvV2Value::Scalar(SecretString::new(v.into())))
            }

            fn visit_bool<E: de::Error>(self, v: bool) -> Result<Self::Value, E> {
                Ok(KvV2Value::Scalar(SecretString::new(v.to_string().into())))
            }

            fn visit_i64<E: de::Error>(self, v: i64) -> Result<Self::Value, E> {
                Ok(KvV2Value::Scalar(SecretString::new(v.to_string().into())))
            }

            fn visit_u64<E: de::Error>(self, v: u64) -> Result<Self::Value, E> {
                Ok(KvV2Value::Scalar(SecretString::new(v.to_string().into())))
            }

            fn visit_f64<E: de::Error>(self, v: f64) -> Result<Self::Value, E> {
                Ok(KvV2Value::Scalar(SecretString::new(v.to_string().into())))
            }

            fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
                Ok(KvV2Value::Unsupported)
            }

            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
                while seq.next_element::<IgnoredAny>()?.is_some() {}
                Ok(KvV2Value::Unsupported)
            }

            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                while map.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {}
                Ok(KvV2Value::Unsupported)
            }
        }

        deserializer.deserialize_any(KvV2ValueVisitor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_kv_scalars_deserialize_as_secrets() {
        let json = r#"{"password": "hunter2", "port": 5432, "enabled": true, "ratio": 1.5}"#;
        let fields: HashMap<String, KvV2Value> = serde_json::from_str(json).unwrap();

        let expect_secret = |key: &str| match &fields[key] {
            KvV2Value::Scalar(s) => s.expose_secret().to_string(),
            KvV2Value::Unsupported => panic!("field '{key}' should be a scalar"),
        };

        assert_eq!(expect_secret("password"), "hunter2");
        assert_eq!(expect_secret("port"), "5432");
        assert_eq!(expect_secret("enabled"), "true");
        assert_eq!(expect_secret("ratio"), "1.5");
    }

    #[test]
    fn test_kv_structured_values_unsupported() {
        let json = r#"{"nothing": null, "list": [1, 2], "nested": {"a": 1}}"#;
        let fields: HashMap<String, KvV2Value> = serde_json::from_str(json).unwrap();

        for key in ["nothing", "list", "nested"] {
            assert!(matches!(fields[key], KvV2Value::Unsupported));
        }
    }
}
