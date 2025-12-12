use crate::compose::ComposeMsg;
use crate::env::EnvManager;
use crate::provider::Provider;
use crate::secrets::Secret;
use clap::Args;
use secrecy::ExposeSecret;
use std::sync::Arc;

fn parse_secret_path(s: &str) -> Result<Secret, String> {
    Secret::from_file(s).map_err(|e| e.to_string())
}

#[derive(Args, Debug)]
pub struct UpArgs {
    /// Provider configuration
    #[command(flatten)]
    pub provider: Provider,

    /// Files containing environment variables which may contain secret references
    #[arg(
        env = "LOCKET_ENV_FILE",
        value_name = "/path/to/.env",
        value_delimiter = ',',
        hide_env_values = true,
        help_heading = None,
        value_parser = parse_secret_path,
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

    let mut secrets = Vec::with_capacity(args.env_file.len() + args.env.len());

    secrets.extend(args.env_file);
    secrets.extend(args.env);

    let manager = EnvManager::new(secrets, provider);

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
