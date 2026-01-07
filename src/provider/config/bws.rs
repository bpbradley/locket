use crate::provider::{AuthToken, ConcurrencyLimit, bws::BwsUrl};
use clap::Args;
use locket_derive::Overlay;
use serde::Deserialize;

#[derive(Args, Debug, Clone, Overlay, Deserialize, Default)]
#[locket(config = "BwsConfig")]
pub struct BwsArgs {
    /// Bitwarden API URL
    #[arg(long = "bws.api", env = "BWS_API_URL")]
    #[locket(default = "https://api.bitwarden.com")]
    api_url: Option<BwsUrl>,

    /// Bitwarden Identity URL
    #[arg(long = "bws.identity", env = "BWS_IDENTITY_URL")]
    #[locket(default = "https://identity.bitwarden.com")]
    identity_url: Option<BwsUrl>,

    /// Maximum number of concurrent requests to Bitwarden Secrets Manager
    #[arg(long = "bws.max-concurrent", env = "BWS_MAX_CONCURRENT")]
    #[locket(default = ConcurrencyLimit::new(20))]
    max_concurrent: Option<ConcurrencyLimit>,

    /// BWS User Agent
    #[arg(long = "bws.user-agent", env = "BWS_USER_AGENT")]
    #[locket(default = env!("CARGO_PKG_NAME"))]
    user_agent: Option<String>,

    /// Bitwarden Machine Token
    ///
    /// Either provide the token directly or via a file with `file:` prefix
    #[arg(long = "bws.token", env = "BWS_MACHINE_TOKEN", hide_env_values = true)]
    token: Option<AuthToken>,
}

#[derive(Debug, Clone)]
pub struct BwsConfig {
    pub api_url: BwsUrl,
    pub identity_url: BwsUrl,
    pub max_concurrent: ConcurrencyLimit,
    pub user_agent: String,
    pub token: AuthToken,
}
