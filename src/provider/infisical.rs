//! Infisical provider implementation.

use super::{
    ConcurrencyLimit, ProviderError, SecretsProvider,
    config::infisical::InfisicalConfig,
    references::{
        InfisicalParseError, InfisicalPath, InfisicalProjectId, InfisicalReference,
        InfisicalSecretType, InfisicalSlug, ReferenceParser, SecretReference,
    },
};
use async_trait::async_trait;
use futures::{StreamExt, stream};
use reqwest::{Client, StatusCode};
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::str::FromStr;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tracing::{debug, warn};
use url::Url;
use uuid::Uuid;

pub struct InfisicalProvider {
    client: Client,
    config: ProviderConfig,
    auth: InfisicalAuthenticator,
}

impl InfisicalProvider {
    pub async fn new(config: InfisicalConfig) -> Result<Self, ProviderError> {
        let (auth_config, config) = config.into();

        let client = Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .map_err(|e| ProviderError::Other(e.to_string()))?;

        let auth = InfisicalAuthenticator::try_new(client.clone(), auth_config).await?;

        Ok(Self {
            client,
            config,
            auth,
        })
    }

    async fn fetch(&self, reference: &InfisicalReference) -> Result<SecretString, ProviderError> {
        let environment = reference
            .options
            .env
            .as_ref()
            .or(self.config.default_env.as_ref())
            .ok_or_else(|| {
                ProviderError::InvalidConfig(format!(
                    "Missing environment for secret '{}' and no default provided",
                    reference.key
                ))
            })?;

        let project_id = reference
            .options
            .project_id
            .as_ref()
            .or(self.config.default_project.as_ref())
            .ok_or_else(|| {
                ProviderError::InvalidConfig(format!(
                    "Missing project_id for secret '{}' and no default provided",
                    reference.key
                ))
            })?;

        let secret_path: &InfisicalPath = reference
            .options
            .path
            .as_ref()
            .unwrap_or(&self.config.default_path);

        let secret_type: InfisicalSecretType = reference
            .options
            .secret_type
            .unwrap_or(self.config.default_secret_type);

        let secret_name = reference.key.as_str();

        let url = self
            .config
            .url
            .join(&format!("/api/v4/secrets/{}", secret_name))
            .map_err(ProviderError::Url)?;

        let query_params = SecretQueryParams {
            project_id,
            environment,
            secret_path,
            secret_type,
            expand_secret_references: true,
            include_imports: true,
        };

        let token = self.auth.get_token().await?;

        let resp = self
            .client
            .get(url)
            .query(&query_params)
            .bearer_auth(token.expose_secret())
            .send()
            .await
            .map_err(|e| ProviderError::Network(Box::new(e)))?;

        match resp.status() {
            StatusCode::OK => {
                let wrapper: InfisicalSecretResponse = resp
                    .json()
                    .await
                    .map_err(|e| ProviderError::Network(Box::new(e)))?;
                Ok(wrapper.secret.secret_value)
            }
            StatusCode::NOT_FOUND => Err(ProviderError::NotFound(reference.to_string())),
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => Err(ProviderError::Unauthorized(
                format!("Access denied for {}", reference),
            )),
            s => {
                let txt = resp.text().await.unwrap_or_default();
                Err(ProviderError::Other(format!(
                    "Infisical error {}: {}",
                    s, txt
                )))
            }
        }
    }
}

impl ReferenceParser for InfisicalProvider {
    fn parse(&self, raw: &str) -> Option<SecretReference> {
        match InfisicalReference::from_str(raw) {
            Ok(reference) => Some(SecretReference::Infisical(reference)),
            Err(InfisicalParseError::InvalidScheme) => None,
            Err(e) => {
                warn!("Invalid reference '{}': {}", raw, e);
                None
            }
        }
    }
}

#[async_trait]
impl SecretsProvider for InfisicalProvider {
    async fn fetch_map(
        &self,
        references: &[SecretReference],
    ) -> Result<HashMap<SecretReference, SecretString>, ProviderError> {
        let refs: Vec<&InfisicalReference> = references
            .iter()
            .filter_map(|r| r.try_into().ok())
            .collect();

        if refs.is_empty() {
            return Ok(HashMap::new());
        }

        let results = stream::iter(refs.into_iter().cloned())
            .map(|ir| async move {
                match self.fetch(&ir).await {
                    Ok(val) => Ok(Some((SecretReference::Infisical(ir), val))),
                    Err(ProviderError::NotFound(_)) => Ok(None),
                    Err(e) => Err(e),
                }
            })
            .buffer_unordered(self.config.max_concurrent.into_inner())
            .collect::<Vec<_>>()
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

struct InfisicalAuthenticator {
    client: Client,
    config: AuthConfig,
    token: RwLock<InfisicalToken>,
}

impl InfisicalAuthenticator {
    pub async fn try_new(client: Client, config: AuthConfig) -> Result<Self, ProviderError> {
        let token = Self::login(&client, &config).await?;

        Ok(Self {
            client,
            config,
            token: RwLock::new(token),
        })
    }

    /// Returns a valid bearer token, renewing it if necessary.
    pub async fn get_token(&self) -> Result<SecretString, ProviderError> {
        {
            let guard = self.token.read().await;
            if !guard.is_expired() {
                return Ok(guard.access_token.clone());
            }
        }

        // Token expired. Need to renew
        let mut guard = self.token.write().await;

        // Check if token is expired again in case it was renewed by another thread
        // while waiting for the write lock
        if !guard.is_expired() {
            return Ok(guard.access_token.clone());
        }

        debug!("Token expired. Renewing...");
        let new_token = Self::login(&self.client, &self.config).await?;

        *guard = new_token.clone();

        Ok(new_token.access_token)
    }

    async fn login(client: &Client, config: &AuthConfig) -> Result<InfisicalToken, ProviderError> {
        let url = config
            .url
            .join("/api/v1/auth/universal-auth/login")
            .map_err(ProviderError::Url)?;

        let payload = LoginParams {
            client_id: &config.client_id,
            client_secret: ClientSecretView(&config.client_secret),
        };

        let resp = client
            .post(url)
            .json(&payload)
            .send()
            .await
            .map_err(|e| ProviderError::Network(Box::new(e)))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(ProviderError::Unauthorized(format!(
                "Infisical login failed: {} - {}",
                status, text
            )));
        }

        let login_resp: LoginResponse = resp
            .json()
            .await
            .map_err(|e| ProviderError::Network(Box::new(e)))?;

        debug!(
            "Login successful. Expires in {} seconds",
            login_resp.expires_in
        );

        Ok(InfisicalToken {
            access_token: login_resp.access_token,
            expires_at: Instant::now() + Duration::from_secs(login_resp.expires_in),
        })
    }
}

#[derive(Debug, Clone)]
struct AuthConfig {
    url: Url,
    client_id: Uuid,
    client_secret: SecretString,
}

#[derive(Debug, Clone)]
struct ProviderConfig {
    url: Url,
    default_path: InfisicalPath,
    default_secret_type: InfisicalSecretType,
    default_env: Option<InfisicalSlug>,
    default_project: Option<InfisicalProjectId>,
    max_concurrent: ConcurrencyLimit,
}

impl From<InfisicalConfig> for (AuthConfig, ProviderConfig) {
    fn from(config: InfisicalConfig) -> Self {
        let auth_config = AuthConfig {
            url: config.infisical_url.clone(),
            client_id: config.infisical_client_id,
            client_secret: config.infisical_client_secret.into(),
        };

        let provider_config = ProviderConfig {
            url: config.infisical_url,
            default_env: config.infisical_default_environment,
            default_project: config.infisical_default_project_id,
            default_path: config.infisical_default_path,
            default_secret_type: config.infisical_default_secret_type,
            max_concurrent: config.infisical_max_concurrent,
        };

        (auth_config, provider_config)
    }
}

#[derive(Debug, Clone)]
struct InfisicalToken {
    access_token: SecretString,
    expires_at: Instant,
}

impl InfisicalToken {
    fn is_expired(&self) -> bool {
        self.expires_at <= Instant::now() + Duration::from_secs(60)
    }
}

struct ClientSecretView<'a>(&'a SecretString);

impl<'a> Serialize for ClientSecretView<'a> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(self.0.expose_secret())
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SecretQueryParams<'a> {
    project_id: &'a InfisicalProjectId,
    environment: &'a InfisicalSlug,
    secret_path: &'a InfisicalPath,

    #[serde(rename = "type")]
    secret_type: InfisicalSecretType,

    expand_secret_references: bool,
    include_imports: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct InfisicalSecretResponse {
    secret: InfisicalSecret,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct InfisicalSecret {
    secret_value: SecretString,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LoginResponse {
    access_token: SecretString,
    expires_in: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LoginParams<'a> {
    client_id: &'a Uuid,
    client_secret: ClientSecretView<'a>,
}
