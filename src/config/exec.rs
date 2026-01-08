use crate::logging::{Logger, LoggerArgs};
use crate::process::{ProcessTimeout, ShellCommand};
use crate::provider::{Provider, ProviderArgs};
use crate::secrets::{Secret, SecretManagerArgs, SecretManagerConfig};
use crate::watch::DebounceDuration;
use clap::Args;
use locket_derive::LayeredConfig;
use serde::Deserialize;

#[derive(Debug, Clone)]
pub struct ExecConfig {
    pub cmd: ShellCommand,
    pub watch: bool,
    pub interactive: Option<bool>,
    pub env_files: Vec<Secret>,
    pub env_overrides: Vec<Secret>,
    pub manager: SecretManagerConfig,
    pub provider: Provider,
    pub timeout: ProcessTimeout,
    pub debounce: DebounceDuration,
    pub logger: Logger,
}

#[derive(Args, Debug, Clone, Default, Deserialize, LayeredConfig)]
#[serde(rename_all = "kebab-case")]
#[locket(try_into = "ExecConfig")]
pub struct ExecArgs {
    /// Watch mode will monitor for changes to .env files and restart the command if changes are detected.
    #[arg(long, env = "LOCKET_EXEC_WATCH")]
    #[locket(default = false)]
    pub watch: Option<bool>,

    /// Run the command in interactive mode, attaching stdin/stdout/stderr.
    ///
    /// If not specified, defaults to true in non-watch mode and false in watch mode.
    #[arg(
        long,
        env = "LOCKET_EXEC_INTERACTIVE",
        num_args = 0..=1,
        default_missing_value = "true",
        require_equals = true,
    )]
    #[locket(optional)]
    pub interactive: Option<bool>,

    /// Files containing environment variables which may contain secret references
    #[arg(
        long = "env-files",
        env = "LOCKET_ENV_FILE",
        value_name = "/path/to/.env",
        alias = "env-file",
        value_delimiter = ',',
        hide_env_values = true,
        help_heading = None,
        value_parser = crate::path::parse_secret_path,
        action = clap::ArgAction::Append,
    )]
    pub env_files: Vec<Secret>,

    /// Environment variable overrides which may contain secret references
    #[arg(
        long = "env-overrides",
        short = 'e',
        env = "LOCKET_ENV",
        value_name = "KEY=VAL, KEY=@FILE or /path/to/.env",
        value_delimiter = ',',
        hide_env_values = true,
        alias = "env",
        help_heading = None,
        action = clap::ArgAction::Append,
    )]
    pub env_overrides: Vec<Secret>,

    #[command(flatten)]
    #[serde(flatten)]
    pub manager: SecretManagerArgs,

    /// Timeout duration for process termination signals.
    ///
    /// Unitless numbers are interpreted as seconds.
    #[arg(long, env = "LOCKET_EXEC_TIMEOUT")]
    #[locket(default = ProcessTimeout::default())]
    pub timeout: Option<ProcessTimeout>,

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
    pub provider: ProviderArgs,

    /// Command to execute with secrets injected into environment
    ///
    /// Must be the last argument(s), following a `--` separator.
    ///
    /// Example: `locket exec -e locket.env -- docker compose up -d`
    #[arg(required = true, trailing_var_arg = true, help_heading = None)]
    #[locket(try_into)]
    pub cmd: Vec<String>,
}
