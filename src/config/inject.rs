use crate::health::StatusFile;
use crate::logging::{Logger, LoggerArgs};
use crate::path::AbsolutePath;
use crate::provider::{Provider, ProviderArgs};
use crate::secrets::{SecretManagerArgs, SecretManagerConfig};
use crate::watch::DebounceDuration;
use clap::{Args, ValueEnum};
use locket_derive::Overlay;
use serde::Deserialize;

#[derive(Default, Copy, Clone, Debug, ValueEnum, Deserialize, PartialEq, Eq)]
pub enum InjectMode {
    #[default]
    /// **Default** Materialize all secrets once and exit
    OneShot,
    /// **Docker Default** Watch for changes on templates and reinject
    Watch,
    /// Inject once and then park to keep the process alive
    Park,
}

#[derive(Debug, Clone)]
pub struct InjectConfig {
    pub mode: InjectMode,
    pub status_file: Option<StatusFile>,
    pub manager: SecretManagerConfig,
    pub provider: Provider,
    pub debounce: DebounceDuration,
    pub logger: Logger,
}

#[derive(Args, Debug, Clone, Default, Deserialize, Overlay)]
#[locket(try_into = "InjectConfig")]
pub struct InjectArgs {
    /// Path to configuration file
    #[arg(long, env = "LOCKET_CONFIG")]
    #[serde(skip)]
    #[locket(skip)]
    pub config: Option<AbsolutePath>,

    /// Mode of operation
    #[arg(long = "mode", env = "LOCKET_INJECT_MODE", value_enum)]
    #[locket(default = InjectMode::OneShot)]
    pub mode: Option<InjectMode>,

    /// Status file path used for healthchecks.
    ///
    /// If not provided, no status file is created.
    ///
    /// **Docker Default:** `/dev/shm/locket/ready`
    #[arg(long = "status-file", env = "LOCKET_STATUS_FILE")]
    #[locket(optional)]
    pub status_file: Option<StatusFile>,

    /// Secret Management Configuration
    #[command(flatten)]
    #[serde(flatten)]
    pub manager: SecretManagerArgs,

    /// Debounce duration for filesystem events in watch mode.
    ///
    /// Events occurring within this duration will be coalesced into a single update
    /// so as to not overwhelm the secrets manager with rapid successive updates from
    /// filesystem noise.
    ///
    /// Handles human-readable strings like "100ms", "2s", etc.
    /// Unitless numbers are interpreted as milliseconds.
    #[arg(long, env = "WATCH_DEBOUNCE")]
    #[locket(default = DebounceDuration::default())]
    pub debounce: Option<DebounceDuration>,

    /// Logging configuration
    #[command(flatten)]
    #[serde(flatten)]
    #[locket(default)]
    pub logger: LoggerArgs,

    /// Secrets provider selection
    #[command(flatten, next_help_heading = "Provider Configuration")]
    #[serde(flatten)]
    #[locket(try_into)]
    pub provider: ProviderArgs,
}
