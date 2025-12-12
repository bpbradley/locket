use crate::compose::ComposeMsg;
use crate::env::EnvManager;
use crate::provider::Provider;
use clap::Args;
use secrecy::ExposeSecret;
use std::sync::Arc;

#[derive(Args, Debug)]
pub struct UpArgs {
    /// Provider configuration
    #[command(flatten)]
    pub provider: Provider,

    #[arg(
        long = "env_file",
        env = "LOCKET_ENV_FILE",
        value_name = "KEY=VAL or @FILE",
        value_delimiter = ',',
        hide_env_values = true,
        help_heading = None,
    )]
    pub env_file: Option<EnvFile>,

    /// Secrets to be injected as environment variables
    #[arg(
        long,
        short = 'e',
        env = "LOCKET_ENV",
        value_name = "KEY=VAL or @FILE",
        value_delimiter = ',',
        hide_env_values = true,
        help_heading = None,
    )]
    pub env: Vec<EnvSource>,

    /// Service name from Docker Compose
    #[arg(help_heading = None)]
    pub service: String,
}

pub async fn up(project: String, args: UpArgs) -> sysexits::ExitCode {
    ComposeMsg::info(format!("Starting project: {}", project));

    let provider = match args.provider.build().await {
        Ok(p) => Arc::from(p),
        Err(e) => {
            ComposeMsg::error(format!("Failed to initialize provider: {}", e));
            return sysexits::ExitCode::Config;
        }
    };

    let mut envs = Vec::new();

    if let Some(env_file) = args.env_file.clone() {
        envs.push(EnvSource::File(env_file));
    }
    envs.extend(args.env);

    let manager = EnvManager::new(envs, provider);

    let env = match manager.resolve().await {
        Ok(map) => map,
        Err(e) => {
            ComposeMsg::error(format!("Failed to resolve environment: {}", e));
            return sysexits::ExitCode::Unavailable;
        }
    };

    for (key, value) in env {
        ComposeMsg::set_env(&key, value.expose_secret());
        ComposeMsg::debug(format!("Injected secret: {}", key));
    }

    sysexits::ExitCode::Ok
}
