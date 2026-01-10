use crate::health::StatusFile;
use crate::logging::{Logger, LoggerArgs};
use crate::provider::{Provider, ProviderArgs};
use crate::secrets::{SecretManagerArgs, SecretManagerConfig};
use crate::watch::DebounceDuration;
use clap::{Args, ValueEnum};
use locket_derive::LayeredConfig;
use serde::{Deserialize, Serialize};

#[derive(Default, Copy, Clone, Debug, ValueEnum, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum InjectMode {
    #[default]
    /// **Default** Materialize all secrets once and exit
    OneShot,
    /// **Docker Default** Watch for changes on templates and reinject
    Watch,
    /// Inject once and then park to keep the process alive
    Park,
}

impl std::fmt::Display for InjectMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.to_possible_value()
            .expect("no values are skipped")
            .get_name()
            .fmt(f)
    }
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

#[derive(Args, Debug, Clone, Default, Serialize, Deserialize, LayeredConfig)]
#[serde(rename_all = "kebab-case")]
#[locket(try_into = "InjectConfig", section = "inject")]
pub struct InjectArgs {
    /// Mode of operation
    #[arg(long, env = "LOCKET_INJECT_MODE", value_enum)]
    #[locket(default = InjectMode::OneShot)]
    pub mode: Option<InjectMode>,

    /// Status file path used for healthchecks.
    ///
    /// If not provided, no status file is created.
    ///
    /// **Docker Default:** `/dev/shm/locket/ready`
    #[arg(long, env = "LOCKET_STATUS_FILE")]
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
    pub logger: LoggerArgs,

    /// Secrets provider selection
    #[command(flatten, next_help_heading = "Provider Configuration")]
    #[serde(flatten)]
    #[locket(try_into)]
    pub provider: ProviderArgs,
}
