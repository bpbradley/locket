use crate::cmd::config::compose::UpArgs;
use crate::compose::ComposeMsg;
use crate::env::EnvManager;
use crate::logging::{LogFormat, Logger};
use crate::provider::Provider;

use secrecy::ExposeSecret;
use tracing::{debug, info};

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
