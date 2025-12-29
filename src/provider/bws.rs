//! Bitwarden Secrets Manager provider implementation.
//! This module defines a `BwsProvider` that implements
//! the `SecretsProvider` trait for fetching secrets
//!
//! It uses the official Bitwarden SDK

use super::references::{ReferenceParser, SecretReference};
use crate::provider::{AuthToken, ProviderError, SecretsProvider, macros::define_auth_token};
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
use uuid::Uuid;

define_auth_token!(
    struct_name: BwsToken,
    prefix: "bws",
    env: "BWS_MACHINE_TOKEN",
    group_id: "bws_token",
    doc_string: "Bitwarden Secrets Manager machine token"
);

#[derive(Args, Debug, Clone)]
pub struct BwsConfig {
    /// Bitwarden API URL
    #[arg(
        long = "bws.api",
        env = "BWS_API_URL",
        default_value = "https://api.bitwarden.com"
    )]
    pub api_url: String,

    /// Bitwarden Identity URL
    #[arg(
        long = "bws.identity",
        env = "BWS_IDENTITY_URL",
        default_value = "https://identity.bitwarden.com"
    )]
    pub identity_url: String,

    /// Maximum number of concurrent requests to Bitwarden Secrets Manager
    #[arg(
        long = "bws.max-concurrent",
        env = "BWS_MAX_CONCURRENT",
        default_value_t = 20
    )]
    pub bws_max_concurrent: usize,

    /// BWS User Agent
    #[arg(
        long = "bws.user-agent",
        env = "BWS_USER_AGENT",
        default_value = "locket"
    )]
    pub bws_user_agent: String,

    #[command(flatten)]
    pub token: BwsToken,
}

impl Default for BwsConfig {
    fn default() -> Self {
        Self {
            api_url: "https://api.bitwarden.com".to_string(),
            identity_url: "https://identity.bitwarden.com".to_string(),
            bws_max_concurrent: 20,
            bws_user_agent: "locket".to_string(),
            token: BwsToken::default(),
        }
    }
}

pub struct BwsProvider {
    client: Client,
    max_concurrent: usize,
}

impl ReferenceParser for BwsProvider {
    fn parse(&self, raw: &str) -> Option<SecretReference> {
        uuid::Uuid::parse_str(raw).ok().map(SecretReference::Bws)
    }
}

impl BwsProvider {
    pub async fn new(cfg: BwsConfig) -> Result<Self, ProviderError> {
        let token: AuthToken = cfg.token.try_into()?;
        let settings = ClientSettings {
            identity_url: cfg.identity_url,
            api_url: cfg.api_url,
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
        let client = &self.client;

        let refs: Vec<(SecretReference, Uuid)> = references
            .iter()
            .filter_map(|r| match r {
                SecretReference::Bws(uuid) => Some((r.clone(), *uuid)),
                _ => None,
            })
            .collect();

        if refs.is_empty() {
            return Ok(HashMap::new());
        }

        let results: Vec<Result<(SecretReference, SecretString), ProviderError>> =
            stream::iter(refs)
                .map(|(original_ref, id)| async move {
                    let req = SecretGetRequest { id };
                    let resp = client
                        .secrets()
                        .get(&req)
                        .await
                        .map_err(|e| ProviderError::NotFound(format!("{} ({})", id, e)))?;

                    Ok((original_ref, SecretString::new(resp.value.into())))
                })
                .buffer_unordered(self.max_concurrent)
                .collect()
                .await;

        let mut map = HashMap::new();
        for res in results {
            match res {
                Ok((k, v)) => {
                    map.insert(k, v);
                }
                Err(e) => return Err(e),
            }
        }

        Ok(map)
    }
}
