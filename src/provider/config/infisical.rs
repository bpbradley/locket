use crate::provider::{
    AuthToken, ConcurrencyLimit,
    references::{InfisicalPath, InfisicalProjectId, InfisicalSecretType, InfisicalSlug},
};
use clap::Args;
use locket_derive::LayeredConfig;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub struct InfisicalConfig {
    pub infisical_url: url::Url,
    pub infisical_client_secret: AuthToken,
    pub infisical_client_id: String,
    pub infisical_default_path: InfisicalPath,
    pub infisical_default_secret_type: InfisicalSecretType,
    pub infisical_default_environment: Option<InfisicalSlug>,
    pub infisical_default_project_id: Option<InfisicalProjectId>,
    pub infisical_max_concurrent: ConcurrencyLimit,
}

#[derive(Args, Debug, Clone, Default, LayeredConfig, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
#[locket(try_into = "InfisicalConfig")]
pub struct InfisicalArgs {
    /// The URL of the Infisical instance to connect to.
    #[arg(long, env = "INFISICAL_URL")]
    #[locket(default = "https://us.infisical.com")]
    pub infisical_url: Option<url::Url>,

    /// The client secret for Universal Auth to authenticate with Infisical.
    ///
    /// Either provide the token directly or via a file with `file:` prefix
    #[arg(long, env = "INFISICAL_CLIENT_SECRET", hide_env_values = true)]
    pub infisical_client_secret: Option<AuthToken>,

    /// The client ID for Universal Auth to authenticate with Infisical.
    #[arg(long, env = "INFISICAL_CLIENT_ID")]
    pub infisical_client_id: Option<String>,

    /// The default environment slug to use when one is not specified.
    #[arg(long, env = "INFISICAL_DEFAULT_ENVIRONMENT")]
    pub infisical_default_environment: Option<InfisicalSlug>,

    /// The default project ID to use when one is not specified.
    #[arg(long, env = "INFISICAL_DEFAULT_PROJECT_ID")]
    pub infisical_default_project_id: Option<InfisicalProjectId>,

    /// The default path to use when one is not specified.
    #[arg(long, env = "INFISICAL_DEFAULT_PATH")]
    #[locket(default = "/")]
    pub infisical_default_path: Option<InfisicalPath>,

    /// The default secret type to use when one is not specified.
    #[arg(long, env = "INFISICAL_DEFAULT_SECRET_TYPE")]
    #[locket(default = InfisicalSecretType::Shared)]
    pub infisical_default_secret_type: Option<InfisicalSecretType>,

    /// Maximum allowed concurrent requests to Infisical API.
    #[arg(long, env = "INFISICAL_MAX_CONCURRENT")]
    #[locket(default = ConcurrencyLimit::new(20))]
    pub infisical_max_concurrent: Option<ConcurrencyLimit>,
}
