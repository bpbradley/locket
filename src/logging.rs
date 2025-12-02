use clap::{Args, ValueEnum};
use serde::{Deserialize, Serialize};
use tracing_subscriber::prelude::*;
use tracing_subscriber::{EnvFilter, fmt};

#[derive(Default, Copy, Clone, Debug, Serialize, Deserialize, ValueEnum)]
pub enum LogFormat {
    #[default]
    Text,
    Json,
}
impl LogFormat {
    pub fn as_str(self) -> &'static str {
        match self {
            LogFormat::Text => "text",
            LogFormat::Json => "json",
        }
    }
}

#[derive(Default, Copy, Clone, Debug, Serialize, Deserialize, ValueEnum)]
pub enum LogLevel {
    Trace,
    Debug,
    #[default]
    Info,
    Warn,
    Error,
}
impl LogLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            LogLevel::Trace => "trace",
            LogLevel::Debug => "debug",
            LogLevel::Info => "info",
            LogLevel::Warn => "warn",
            LogLevel::Error => "error",
        }
    }
}

#[derive(Default, Args, Debug, Clone)]
pub struct Logger {
    /// Log format
    #[arg(long, env = "LOCKET_LOG_FORMAT", value_enum, default_value_t = LogFormat::Text)]
    pub log_format: LogFormat,

    /// Log level
    #[arg(long, env = "LOCKET_LOG_LEVEL", value_enum, default_value_t = LogLevel::Info)]
    pub log_level: LogLevel,
}

impl Logger {
    pub fn new(log_format: LogFormat, log_level: LogLevel) -> Self {
        Self {
            log_format,
            log_level,
        }
    }
    fn filter(&self) -> EnvFilter {
        // Respect RUST_LOG if set
        if let Ok(filter) = EnvFilter::try_from_default_env() {
            return filter;
        }
        // Otherwise, scope to requested log level
        let level = self.log_level.as_str();
        let directives = format!("info,locket={}", level);
        EnvFilter::new(directives)
    }
    pub fn init(&self) -> anyhow::Result<()> {
        let filter = self.filter();
        match self.log_format {
            LogFormat::Json => tracing_subscriber::registry()
                .with(filter)
                .with(fmt::layer().json().with_current_span(false))
                .try_init()
                .map_err(|e| anyhow::anyhow!(e.to_string())),
            LogFormat::Text => tracing_subscriber::registry()
                .with(filter)
                .with(fmt::layer().with_target(false))
                .try_init()
                .map_err(|e| anyhow::anyhow!(e.to_string())),
        }
    }
}
