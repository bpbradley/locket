use crate::cmd::config::volume::PluginConfig;
use crate::error::LocketError;
use crate::volume::VolumePlugin;
use tracing::info;

pub async fn volume(config: PluginConfig) -> Result<(), LocketError> {
    config.logger.init()?;
    info!("Starting volume plugin with config: {:#?}", config);
    VolumePlugin::new(config).run().await?;
    info!("Volume plugin exited successfully");
    Ok(())
}
