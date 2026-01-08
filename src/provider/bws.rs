//! Bitwarden Secrets Manager provider implementation.
//! This module defines a `BwsProvider` that implements
//! the `SecretsProvider` trait for fetching secrets
//!
//! It uses the official Bitwarden SDK

use super::ConcurrencyLimit;
use super::references::{ReferenceParser, SecretReference};
use crate::provider::config::bws::BwsConfig;
use crate::provider::references::BwsReference;
use crate::provider::{ProviderError, SecretsProvider};
use async_trait::async_trait;
use bitwarden::{
    Client,
    auth::login::AccessTokenLoginRequest,
    client::client_settings::{ClientSettings, DeviceType},
    secrets_manager::{ClientSecretsExt, secrets::SecretGetRequest},
};
use futures::stream::{self, StreamExt};
use secrecy::ExposeSecret;
use secrecy::SecretString;
use serde::Deserialize;
use std::collections::HashMap;
use std::str::FromStr;
use url::Url;
use uuid::Uuid;

/// BWS SDK URL wrapper
/// Used to ensure proper URL formatting. BWS SDK accepts a raw string, and fails to parse URLs with trailing slashes
/// This wrapper will ensure proper url encoding at config time, and remove the trailing slash if present when displaying.
#[derive(Debug, Clone, Deserialize)]
pub struct BwsUrl(Url);

impl BwsUrl {
    /// Get the URL as a string, stripping any trailing slash
    /// This is needed because the BWS SDK does not accept URLs with trailing slashes
    pub fn as_bws_string(&self) -> &str {
        let s = self.0.as_str();
        s.strip_suffix('/').unwrap_or(s)
    }
}

impl std::str::FromStr for BwsUrl {
    type Err = ProviderError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(BwsUrl(Url::parse(s)?))
    }
}

impl std::fmt::Display for BwsUrl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_bws_string())
    }
}

impl AsRef<str> for BwsUrl {
    fn as_ref(&self) -> &str {
        self.as_bws_string()
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
        BwsReference::from_str(raw).ok().map(SecretReference::Bws)
    }
}

impl BwsProvider {
    pub async fn new(cfg: BwsConfig) -> Result<Self, ProviderError> {
        let settings = ClientSettings {
            identity_url: cfg.bws_identity_url.to_string(),
            api_url: cfg.bws_api_url.to_string(),
            user_agent: cfg.bws_user_agent,
            device_type: DeviceType::SDK,
        };

        let client = Client::new(Some(settings));

        let auth_req = AccessTokenLoginRequest {
            access_token: cfg.bws_token.expose_secret().to_string(),
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
            .filter_map(|r| {
                r.try_into()
                    .ok()
                    .map(|bws_ref: &BwsReference| (r.clone(), Uuid::from(*bws_ref)))
            })
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
