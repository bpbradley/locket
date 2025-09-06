use tracing_subscriber::prelude::*;
use tracing_subscriber::{fmt, EnvFilter};
use clap::ValueEnum;
use serde::{Deserialize, Serialize};

#[derive(Copy, Clone, Debug, Serialize, Deserialize, ValueEnum)]
pub enum LogFormat {
    Text,
    Json,
}
impl Default for LogFormat {
    fn default() -> Self { LogFormat::Text }
}
impl LogFormat {
    pub fn as_str(self) -> &'static str {
        match self {
            LogFormat::Text => "text",
            LogFormat::Json => "json",
        }
    }
}

#[derive(Copy, Clone, Debug, Serialize, Deserialize, ValueEnum)]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}
impl Default for LogLevel {
    fn default() -> Self { LogLevel::Info }
}
impl LogLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            LogLevel::Trace => "trace",
            LogLevel::Debug => "debug",
            LogLevel::Info  => "info",
            LogLevel::Warn  => "warn",
            LogLevel::Error => "error",
        }
    }
}

pub fn init(format: LogFormat, level: LogLevel) -> anyhow::Result<()> {
    let filter = EnvFilter::try_new(level.as_str()).unwrap_or_else(|_| EnvFilter::new("info"));
    match format {
        LogFormat::Json => {
            tracing_subscriber::registry()
                .with(filter)
                .with(fmt::layer().json().with_current_span(false))
                .try_init()
                .map_err(|e| anyhow::anyhow!(e.to_string()))?;
        }
        LogFormat::Text => {
            tracing_subscriber::registry()
                .with(filter)
                .with(fmt::layer().with_target(false))
                .try_init()
                .map_err(|e| anyhow::anyhow!(e.to_string()))?;
        }
    }
    Ok(())
}
