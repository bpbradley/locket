//! Infisical provider implementation.
//!
//! Uses the Infisical v4 API to fetch secrets
//! and the v1 Univeral Auth API for authentication.
//!
//! The authentication token is lazily refreshed when it expires
//! and it will gracefully handle rotating authentication when request limit is reached

use super::{
    ConcurrencyLimit, ProviderError, SecretsProvider,
    auth::{ExpiringToken, SecretView, TokenAuthenticator, TokenExchange},
    config::infisical::InfisicalConfig,
    references::{
        Extract, InfisicalParseError, InfisicalPath, InfisicalProjectId, InfisicalReference,
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
use std::time::Duration;
use tracing::warn;
use url::Url;
use uuid::Uuid;

pub struct InfisicalProvider {
    client: Client,
    config: ProviderConfig,
    auth: TokenAuthenticator<UniversalAuthLogin>,
}

impl InfisicalProvider {
    pub async fn new(config: InfisicalConfig) -> Result<Self, ProviderError> {
        let secret = config.infisical_client_secret.resolve().await?;
        let auth_config = AuthConfig {
            url: config.infisical_url.clone(),
            client_id: config.infisical_client_id,
            client_secret: secret,
        };

        let provider_config = ProviderConfig::from(config);

        let client = Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .map_err(|e| ProviderError::Other(e.to_string()))?;

        let auth = TokenAuthenticator::try_new(UniversalAuthLogin {
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

        let mut attempt = 0;
        loop {
            attempt += 1;
            let token = self.auth.get_token().await?;

            let resp = self
                .client
                .get(url.clone())
                .query(&query_params)
                .bearer_auth(token.expose_secret())
                .send()
                .await
                .map_err(|e| ProviderError::Network(Box::new(e)))?;

            match resp.status() {
                s if s.is_success() => {
                    let wrapper: InfisicalSecretResponse = resp
                        .json()
                        .await
                        .map_err(|e| ProviderError::Network(Box::new(e)))?;
                    return Ok(wrapper.secret.secret_value);
                }
                // Token may need to be refreshed. Try invalidating the token
                // to trigger a rotation and try again
                StatusCode::UNAUTHORIZED if attempt < 2 => {
                    warn!(
                        "Got Unauthorized for {}. Invalidating token and retrying...",
                        reference.key
                    );
                    self.auth.invalidate(&token).await;
                    continue;
                }
                StatusCode::NOT_FOUND => {
                    return Err(ProviderError::NotFound(reference.to_string()));
                }
                StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
                    return Err(ProviderError::Unauthorized(format!(
                        "Access denied for {}",
                        reference
                    )));
                }
                status => {
                    let txt = resp.text().await.unwrap_or_default();
                    return Err(ProviderError::Other(format!(
                        "Infisical error {}: {}",
                        status, txt
                    )));
                }
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
            .filter_map(InfisicalReference::extract)
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

/// Universal Auth credential exchange for Infisical.
struct UniversalAuthLogin {
    client: Client,
    config: AuthConfig,
}

#[async_trait]
impl TokenExchange for UniversalAuthLogin {
    async fn login(&self) -> Result<ExpiringToken, ProviderError> {
        let url = self
            .config
            .url
            .join("/api/v1/auth/universal-auth/login")
            .map_err(ProviderError::Url)?;

        let payload = LoginParams {
            client_id: &self.config.client_id,
            client_secret: SecretView(&self.config.client_secret),
        };

        let resp = self
            .client
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

        Ok(ExpiringToken::new(
            login_resp.access_token,
            login_resp.expires_in,
        ))
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

impl From<InfisicalConfig> for ProviderConfig {
    fn from(config: InfisicalConfig) -> Self {
        ProviderConfig {
            url: config.infisical_url,
            default_env: config.infisical_default_environment,
            default_project: config.infisical_default_project_id,
            default_path: config.infisical_default_path,
            default_secret_type: config.infisical_default_secret_type,
            max_concurrent: config.infisical_max_concurrent,
        }
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
    client_secret: SecretView<'a>,
}
