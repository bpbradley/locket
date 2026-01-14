//! Infisical provider implementation.

use super::{
    ProviderError, SecretsProvider,
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
use std::time::Duration;
use tracing::warn;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LoginResponse {
    access_token: SecretString,
    #[allow(dead_code)]
    expires_in: i64,
    #[allow(dead_code)]
    token_type: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")] // Automagically handles "workspaceId", "secretPath", etc.
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

pub struct InfisicalProvider {
    client: Client,
    config: InfisicalConfig,
    token: SecretString,
}

impl InfisicalProvider {
    pub async fn new(config: InfisicalConfig) -> Result<Self, ProviderError> {
        let client = Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .map_err(|e| ProviderError::Other(e.to_string()))?;

        // Perform initial login to get access token
        let token = Self::login(&client, &config).await?;

        Ok(Self {
            client,
            config,
            token,
        })
    }

    async fn login(
        client: &Client,
        config: &InfisicalConfig,
    ) -> Result<SecretString, ProviderError> {
        let url = config
            .infisical_url
            .join("/api/v1/auth/universal-auth/login")
            .map_err(ProviderError::Url)?;

        let payload = serde_json::json!({
            "clientId": config.infisical_client_id,
            "clientSecret": config.infisical_client_secret.expose_secret(),
        });

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

        Ok(login_resp.access_token)
    }

    async fn fetch_single(
        &self,
        reference: &InfisicalReference,
    ) -> Result<SecretString, ProviderError> {
        let environment = reference
            .options
            .env
            .as_ref()
            .or(self.config.infisical_default_environment.as_ref())
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
            .or(self.config.infisical_default_project_id.as_ref())
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
            .unwrap_or(&self.config.infisical_default_path);

        let secret_type: InfisicalSecretType = reference
            .options
            .secret_type
            .unwrap_or(self.config.infisical_default_secret_type);

        let secret_name = reference.key.as_str();

        let url = self
            .config
            .infisical_url
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

        let resp = self
            .client
            .get(url)
            .query(&query_params)
            .bearer_auth(self.token.expose_secret())
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
                match self.fetch_single(&ir).await {
                    Ok(val) => Ok(Some((SecretReference::Infisical(ir), val))),
                    Err(ProviderError::NotFound(_)) => Ok(None),
                    Err(e) => Err(e),
                }
            })
            .buffer_unordered(self.config.infisical_max_concurrent.into_inner())
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
