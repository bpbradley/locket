use crate::{
    env::EnvManager,
    error::LocketError,
    events::EventHandler,
    logging::Logger,
    process::{ProcessManager, ProcessTimeout, ShellCommand},
    provider::{Provider, ProviderArgs},
    secrets::Secret,
    watch::{DebounceDuration, FsWatcher},
};
use clap::Args;
use tracing::{debug, info};

#[derive(Args, Debug)]
pub struct ExecArgs {
    /// Watch mode will monitor for changes to .env files and restart the command if changes are detected.
    #[arg(long, env = "LOCKET_EXEC_WATCH", default_value_t = false)]
    pub watch: bool,

    /// Run the command in interactive mode, attaching stdin/stdout/stderr.
    /// If not specified, defaults to true in non-watch mode and false in watch mode.
    #[arg(
        long,
        env = "LOCKET_EXEC_INTERACTIVE",
        num_args = 0..=1,
        default_missing_value = "true",
        require_equals = true,
    )]
    pub interactive: Option<bool>,

    /// Files containing environment variables which may contain secret references
    #[arg(
        long,
        env = "LOCKET_ENV_FILE",
        value_name = "/path/to/.env",
        alias = "env_file",
        value_delimiter = ',',
        hide_env_values = true,
        help_heading = None,
        value_parser = crate::path::parse_secret_path,
        action = clap::ArgAction::Append,
    )]
    pub env_file: Vec<Secret>,

    /// Environment variable overrides which may contain secret references
    #[arg(
        long,
        short = 'e',
        env = "LOCKET_ENV",
        value_name = "KEY=VAL, KEY=@FILE or /path/to/.env",
        value_delimiter = ',',
        hide_env_values = true,
        help_heading = None,
        action = clap::ArgAction::Append,
    )]
    pub env: Vec<Secret>,

    /// Timeout duration for process termination signals.
    /// Unitless numbers are interpreted as seconds.
    #[arg(
        long,
        env = "LOCKET_EXEC_TIMEOUT",
        default_value_t = ProcessTimeout::default(),
    )]
    pub timeout: ProcessTimeout,

    /// Debounce duration for filesystem events in watch mode.
    /// Events occurring within this duration will be coalesced into a single update
    /// so as to not overwhelm the secrets manager with rapid successive updates from
    /// filesystem noise. Handles human-readable strings like "100ms", "2s", etc.
    /// Unitless numbers are interpreted as milliseconds.
    #[arg(long, env = "WATCH_DEBOUNCE", default_value_t = DebounceDuration::default())]
    debounce: DebounceDuration,

    /// Logging configuration
    #[command(flatten)]
    pub logger: Logger,

    /// Secrets provider selection
    #[command(flatten, next_help_heading = "Provider Configuration")]
    provider: ProviderArgs,

    /// Command to execute with secrets injected into environment
    /// Must be the last argument(s), following a `--` separator.
    /// Example: locket exec -e locket.env -- docker compose up -d
    #[arg(required = true, trailing_var_arg = true, help_heading = None)]
    pub cmd: Vec<String>,
}

pub async fn exec(args: ExecArgs) -> Result<(), LocketError> {
    args.logger.init()?;
    info!(
        "Starting locket v{} `exec` service ",
        env!("CARGO_PKG_VERSION")
    );
    debug!("effective config: {:#?}", args);

    // Initialize Provider
    let provider = Provider::from(args.provider).build().await?;

    // Initialize EnvManager
    let mut env_secrets = args.env;
    env_secrets.extend(args.env_file);
    let env_manager = EnvManager::new(env_secrets, provider);

    // Initialize ProcessManager
    let interactive = args.interactive.unwrap_or(!args.watch);
    let command = ShellCommand::try_from(args.cmd)?;
    let mut handler = ProcessManager::new(env_manager, command, interactive, args.timeout);

    // Initial Start
    // We must start the process at least once regardless of mode.
    info!("resolving environment and starting process...");
    handler.start().await?;

    // Execution Mode Branch
    if args.watch {
        let watcher = FsWatcher::new(args.debounce, handler);
        // Watcher gives ownership of the handler back when it exits
        // so we can clean up properly.
        handler = watcher.run().await?;
        handler.cleanup().await;
        info!("watch loop terminated gracefully");
        Ok(())
    } else {
        let result = handler.wait().await;
        handler.cleanup().await;
        result.map_err(LocketError::from)
    }
}
