//! Logging configuration for general purpose use, with clap configuration.
//!
//! Supports log format (text or JSON) and log level (trace, debug, info, warn, error).
//! Uses `tracing` and `tracing-subscriber` for implementation.

use clap::{Args, ValueEnum};
use locket_derive::LayeredConfig;
use serde::{Deserialize, Serialize};
use std::str::FromStr;
use thiserror::Error;
use tracing_subscriber::prelude::*;
use tracing_subscriber::{EnvFilter, fmt};

#[derive(Debug, Error)]
pub enum LoggingError {
    #[error("failed to initialize logging: {0}")]
    Init(#[from] tracing_subscriber::util::TryInitError),
}

#[derive(Default, Copy, Clone, Debug, Serialize, Deserialize, ValueEnum)]
pub enum LogFormat {
    #[default]
    /// Plain text log format
    Text,
    /// JSON log format
    Json,
    #[cfg(feature = "compose")]
    /// Special format for Docker Compose Provider specification
    Compose,
}

impl LogFormat {
    pub fn as_str(self) -> &'static str {
        match self {
            LogFormat::Text => "text",
            LogFormat::Json => "json",
            #[cfg(feature = "compose")]
            LogFormat::Compose => "compose",
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

impl FromStr for LogLevel {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "trace" => Ok(LogLevel::Trace),
            "debug" => Ok(LogLevel::Debug),
            "info" => Ok(LogLevel::Info),
            "warn" => Ok(LogLevel::Warn),
            "error" => Ok(LogLevel::Error),
            _ => Err(format!("Invalid log level: {}", s)),
        }
    }
}

impl FromStr for LogFormat {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "text" => Ok(LogFormat::Text),
            "json" => Ok(LogFormat::Json),
            #[cfg(feature = "compose")]
            "compose" => Ok(LogFormat::Compose),
            _ => Err(format!("Invalid log format: {}", s)),
        }
    }
}

#[derive(Args, Debug, Clone, Default, Deserialize, LayeredConfig)]
#[locket(try_into = "Logger")]
pub struct LoggerArgs {
    /// Log format
    #[arg(long, env = "LOCKET_LOG_FORMAT")]
    #[locket(default = LogFormat::Text)]
    pub log_format: Option<LogFormat>,

    /// Log level
    #[arg(long, env = "LOCKET_LOG_LEVEL")]
    #[locket(default = LogLevel::Info)]
    pub log_level: Option<LogLevel>,
}

#[derive(Default, Args, Debug, Clone, Deserialize)]
pub struct Logger {
    pub log_format: LogFormat,
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
        let requested_level = if let Ok(rust_log) = std::env::var("RUST_LOG") {
            // If the user provides a complex filter (e.g. "locket=debug,hyper=warn"),
            // we trust they know what they are doing and respect it.
            if rust_log.contains(',') || rust_log.contains('=') {
                return EnvFilter::new(rust_log);
            }
            rust_log
        } else {
            // Fallback to CLI args
            self.log_level.as_str().to_string()
        };
        let directives = format!("info,locket={}", requested_level);
        EnvFilter::new(directives)
    }
    pub fn init(&self) -> Result<(), LoggingError> {
        let filter = self.filter();
        match self.log_format {
            LogFormat::Json => tracing_subscriber::registry()
                .with(filter)
                .with(fmt::layer().json().with_current_span(false))
                .try_init()
                .map_err(LoggingError::from),
            LogFormat::Text => tracing_subscriber::registry()
                .with(filter)
                .with(fmt::layer().with_target(false))
                .try_init()
                .map_err(LoggingError::from),
            #[cfg(feature = "compose")]
            LogFormat::Compose => tracing_subscriber::registry()
                .with(filter)
                .with(fmt::layer().event_format(crate::compose::ComposeFormatter))
                .try_init()
                .map_err(LoggingError::from),
        }
    }
}
