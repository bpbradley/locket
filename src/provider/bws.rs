//! Bitwarden Secrets Manager provider implementation.
//! This module defines a `BwsProvider` that implements
//! the `SecretsProvider` trait for fetching secrets
//!
//! It uses the official Bitwarden SDK

use super::ConcurrencyLimit;
use super::references::{ReferenceParser, SecretReference};
use crate::provider::{AuthToken, ProviderError, SecretsProvider};
use async_trait::async_trait;
use bitwarden::{
    Client,
    auth::login::AccessTokenLoginRequest,
    client::client_settings::{ClientSettings, DeviceType},
    secrets_manager::{ClientSecretsExt, secrets::SecretGetRequest},
};
use clap::Args;
use futures::stream::{self, StreamExt};
use secrecy::ExposeSecret;
use secrecy::SecretString;
use std::collections::HashMap;
use url::Url;
use uuid::Uuid;

#[derive(Args, Debug, Clone)]
pub struct BwsConfig {
    /// Bitwarden API URL
    #[arg(
        long = "bws.api",
        env = "BWS_API_URL",
        default_value = "https://api.bitwarden.com"
    )]
    api_url: BwsUrl,

    /// Bitwarden Identity URL
    #[arg(
        long = "bws.identity",
        env = "BWS_IDENTITY_URL",
        default_value = "https://identity.bitwarden.com"
    )]
    identity_url: BwsUrl,

    /// Maximum number of concurrent requests to Bitwarden Secrets Manager
    #[arg(
        long = "bws.max-concurrent",
        env = "BWS_MAX_CONCURRENT",
        default_value_t = ConcurrencyLimit::new(20)
    )]
    bws_max_concurrent: ConcurrencyLimit,

    /// BWS User Agent
    #[arg(
        long = "bws.user-agent",
        env = "BWS_USER_AGENT",
        default_value = "locket"
    )]
    bws_user_agent: String,

    /// Bitwarden Machine Token
    /// Either provide the token directly or via a file with `file:` prefix
    #[arg(long = "bws.token", env = "BWS_MACHINE_TOKEN", hide_env_values = true)]
    token: Option<AuthToken>,
}

/// BWS SDK URL wrapper
/// Used to ensure proper URL formatting. BWS SDK accepts a raw string, and fails to parse URLs with trailing slashes
/// This wrapper will ensure proper url encoding at config time, and remove the trailing slash if present when displaying.
#[derive(Debug, Clone)]
struct BwsUrl(Url);

impl std::str::FromStr for BwsUrl {
    type Err = ProviderError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(BwsUrl(Url::parse(s)?))
    }
}

impl std::fmt::Display for BwsUrl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = self.0.as_str();
        if let Some(stripped) = s.strip_suffix('/') {
            write!(f, "{}", stripped)
        } else {
            write!(f, "{}", s)
        }
    }
}

impl From<Url> for BwsUrl {
    fn from(url: Url) -> Self {
        BwsUrl(url)
    }
}

pub struct BwsProvider {
    client: Client,
    max_concurrent: ConcurrencyLimit,
}

impl ReferenceParser for BwsProvider {
    fn parse(&self, raw: &str) -> Option<SecretReference> {
        uuid::Uuid::parse_str(raw).ok().map(SecretReference::Bws)
    }
}

impl BwsProvider {
    pub async fn new(cfg: BwsConfig) -> Result<Self, ProviderError> {
        let token = cfg.token.ok_or_else(|| {
            ProviderError::InvalidConfig("missing Bitwarden machine token (bws.token)".to_string())
        })?;
        let settings = ClientSettings {
            identity_url: cfg.identity_url.to_string(),
            api_url: cfg.api_url.to_string(),
            user_agent: cfg.bws_user_agent,
            device_type: DeviceType::SDK,
        };

        let client = Client::new(Some(settings));

        let auth_req = AccessTokenLoginRequest {
            access_token: token.expose_secret().to_string(),
            state_file: None, // We are stateless; no cache file
        };

        client
            .auth()
            .login_access_token(&auth_req)
            .await
            .map_err(|e| ProviderError::Unauthorized(format!("BWS login failed: {:#?}", e)))?;

        Ok(Self {
            client,
            max_concurrent: cfg.bws_max_concurrent,
        })
    }
}

#[async_trait]
impl SecretsProvider for BwsProvider {
    async fn fetch_map(
        &self,
        references: &[SecretReference],
    ) -> Result<HashMap<SecretReference, SecretString>, ProviderError> {
        let refs: Vec<(SecretReference, Uuid)> = references
            .iter()
            .filter_map(|r| r.as_bws().map(|id| (r.clone(), *id)))
            .collect();

        if refs.is_empty() {
            return Ok(HashMap::new());
        }

        let mut map = HashMap::with_capacity(refs.len());
        let client = &self.client;

        let mut stream = stream::iter(refs)
            .map(|(key, id)| async move {
                let req = SecretGetRequest { id };

                let resp = client
                    .secrets()
                    .get(&req)
                    .await
                    .map_err(|e| ProviderError::NotFound(format!("{} ({})", id, e)))?;

                Ok((key, SecretString::new(resp.value.into())))
            })
            .buffer_unordered(self.max_concurrent.into_inner());

        while let Some(result) = stream.next().await {
            match result {
                Ok((key, value)) => {
                    map.insert(key, value);
                }
                Err(e) => return Err(e),
            }
        }

        Ok(map)
    }
}
