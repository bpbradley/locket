use tracing_subscriber::prelude::*;
use tracing_subscriber::{fmt, EnvFilter};

pub fn init(format: &str, level: &str) -> anyhow::Result<()> {
    let filter = EnvFilter::try_new(level).unwrap_or_else(|_| EnvFilter::new("info"));
    match format {
        "json" => {
            tracing_subscriber::registry()
                .with(filter)
                .with(fmt::layer().json().with_current_span(false))
                .try_init()
                .map_err(|e| anyhow::anyhow!(e.to_string()))?;
        }
        _ => {
            tracing_subscriber::registry()
                .with(filter)
                .with(fmt::layer().with_target(false))
                .try_init()
                .map_err(|e| anyhow::anyhow!(e.to_string()))?;
        }
    }
    Ok(())
}
