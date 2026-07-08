use crate::cmd::config::compose::DownArgs;
use crate::logging::{LogFormat, Logger};
use tracing::debug;
pub async fn down(project: String, args: DownArgs) -> Result<(), crate::error::LocketError> {
    Logger::new(LogFormat::Compose, args.log_level).init()?;
    debug!("Stopping project {} with: {:#?}", project, args);
    Ok(())
}
