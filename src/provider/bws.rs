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
use secrecy::{ExposeSecret, SecretString};
use std::collections::HashMap;
use std::path::PathBuf;
use tokio::sync::OnceCell;
use uuid::Uuid;

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
    pub max_concurrent: usize,

    #[command(flatten)]
    pub token: BwsToken,
}

impl Default for BwsConfig {
    fn default() -> Self {
        Self {
            api_url: "https://api.bitwarden.com".to_string(),
            identity_url: "https://identity.bitwarden.com".to_string(),
            max_concurrent: 20,
            token: BwsToken::default(),
        }
    }
}

#[derive(Args, Debug, Clone, Default)]
#[group(id = "bws_token", multiple = false, required = true)]
pub struct BwsToken {
    #[arg(long = "bws.token", env = "BWS_ACCESS_TOKEN", hide_env_values = true)]
    val: Option<SecretString>,

    #[arg(long = "bws.token-file", env = "BWS_ACCESS_TOKEN_FILE")]
    file: Option<PathBuf>,
}

impl BwsToken {
    pub fn resolve(&self) -> Result<AuthToken, ProviderError> {
        AuthToken::try_new(
            self.val.clone(),
            self.file.clone(),
            "Bitwarden Secrets Manager",
        )
    }
}

pub struct BwsProvider {
    config: BwsConfig,
    token: AuthToken,
    // Lazy-initialized client.
    // We use OnceCell because authentication is async, but 'new' is sync.
    // TODO: consider reworking provider trait to allow async initialization.
    // This would allow better error handling on auth failure at startup
    client: OnceCell<Client>,
}

impl BwsProvider {
    pub fn new(cfg: BwsConfig) -> Result<Self, ProviderError> {
        Ok(Self {
            token: cfg.token.resolve()?,
            config: cfg,
            client: OnceCell::new(),
        })
    }

    /// Initializes and authenticates the Bitwarden client.
    /// This runs exactly once, the first time fetch_map is called.
    async fn get_client(&self) -> Result<&Client, ProviderError> {
        self.client
            .get_or_try_init(|| async {
                let settings = ClientSettings {
                    identity_url: self.config.identity_url.clone(),
                    api_url: self.config.api_url.clone(),
                    user_agent: "locket".to_string(),
                    device_type: DeviceType::SDK,
                };

                let client = Client::new(Some(settings));

                let auth_req = AccessTokenLoginRequest {
                    access_token: self.token.expose_secret().to_string(),
                    state_file: None, // We are stateless; no cache file
                };

                client
                    .auth()
                    .login_access_token(&auth_req)
                    .await
                    .map_err(|e| {
                        ProviderError::Unauthorized(format!("Bitwarden login failed: {}", e))
                    })?;

                Ok(client)
            })
            .await
    }
}

#[async_trait]
impl SecretsProvider for BwsProvider {
    fn accepts_key(&self, key: &str) -> bool {
        // BWS native reference is just a UUID.
        // TODO: Maybe we can support a bws:// scheme as well to allow access by name?
        Uuid::parse_str(key).is_ok()
    }

    async fn fetch_map(
        &self,
        references: &[&str],
    ) -> Result<HashMap<String, String>, ProviderError> {
        let client = self.get_client().await?;

        let refs: Vec<String> = references.iter().map(|s| s.to_string()).collect();

        let results: Vec<Result<(String, String), ProviderError>> = stream::iter(refs)
            .map(|key| async move {
                let id = Uuid::parse_str(&key)
                    .map_err(|_| ProviderError::InvalidConfig(format!("invalid uuid: {}", key)))?;

                let req = SecretGetRequest { id };
                let resp = client
                    .secrets()
                    .get(&req)
                    .await
                    .map_err(|e| ProviderError::NotFound(format!("{} ({})", key, e)))?;

                Ok((key, resp.value))
            })
            .buffer_unordered(self.config.max_concurrent)
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
