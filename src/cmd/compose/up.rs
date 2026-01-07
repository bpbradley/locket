use crate::compose::ComposeMsg;
use crate::env::EnvManager;
use crate::logging::{LogFormat, LogLevel, Logger};
use crate::provider::{Provider, ProviderArgs};
use crate::secrets::Secret;
use clap::Args;
use secrecy::ExposeSecret;
use tracing::{debug, info};

#[derive(Args, Debug)]
pub struct UpArgs {
    /// Provider configuration
    #[command(flatten)]
    pub provider: ProviderArgs,

    /// Files containing environment variables which may contain secret references
    #[arg(
        long,
        env = "LOCKET_ENV_FILE",
        value_name = "/path/to/.env",
        alias = "env_file",
        value_delimiter = ',',
        hide_env_values = true,
        help_heading = None,
        value_parser = crate::path::parse_secret_path,
        action = clap::ArgAction::Append,
    )]
    pub env_file: Vec<Secret>,

    /// Environment variable overrides which may contain secret references
    #[arg(
        long,
        short = 'e',
        env = "LOCKET_ENV",
        value_name = "KEY=VAL, KEY=@FILE or /path/to/.env",
        value_delimiter = ',',
        hide_env_values = true,
        help_heading = None,
        action = clap::ArgAction::Append,
    )]
    pub env: Vec<Secret>,

    /// Log level
    #[arg(long, env = "LOCKET_LOG_LEVEL", value_enum, default_value_t = LogLevel::Debug)]
    pub log_level: LogLevel,

    /// Service name from Docker Compose
    #[arg(help_heading = None)]
    pub service: String,
}

pub async fn up(project: String, args: UpArgs) -> Result<(), crate::error::LocketError> {
    Logger::new(LogFormat::Compose, args.log_level).init()?;
    info!("Starting project: {}", project);

    let provider = Provider::try_from(args.provider)?.build().await?;

    let mut secrets = Vec::with_capacity(args.env_file.len() + args.env.len());

    secrets.extend(args.env_file);
    secrets.extend(args.env);

    let manager = EnvManager::new(secrets, provider);

    let env = manager.resolve().await?;

    for (key, value) in env {
        ComposeMsg::set_env(key.as_ref(), value.expose_secret());
        debug!("Injected secret: {}", key.as_ref());
    }

    Ok(())
}
