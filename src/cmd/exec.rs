use crate::{
    env::EnvManager,
    error::LocketError,
    events::{EventHandler, FsEvent, HandlerError},
    logging::Logger,
    path::AbsolutePath,
    process::{ProcessManager, ProcessTimeout, ShellCommand},
    provider::{Provider, ProviderArgs},
    secrets::{Secret, SecretFileManager, SecretFileOpts},
    watch::{DebounceDuration, FsWatcher},
};
use clap::Args;
use futures::future::BoxFuture;
use std::collections::HashSet;
use tracing::{debug, info};

#[derive(Args, Debug)]
pub struct ExecArgs {
    /// Watch mode will monitor for changes to .env files and restart the command if changes are detected.
    #[arg(long, env = "LOCKET_EXEC_WATCH", default_value_t = false)]
    pub watch: bool,

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

    #[command(flatten)]
    pub files: SecretFileOpts,

    /// Timeout duration for process termination signals.
    /// Unitless numbers are interpreted as seconds.
    #[arg(
        long,
        env = "LOCKET_EXEC_TIMEOUT",
        default_value_t = ProcessTimeout::default(),
    )]
    pub timeout: ProcessTimeout,

    /// Debounce duration for filesystem events in watch mode.
    ///
    /// Events occurring within this duration will be coalesced into a single update
    /// so as to not overwhelm the secrets manager with rapid successive updates from
    /// filesystem noise.
    ///
    /// Handles human-readable strings like "100ms", "2s", etc.
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
    ///
    /// Must be the last argument(s), following a `--` separator.
    ///
    /// Example: `locket exec -e locket.env -- docker compose up -d`
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
    let provider = Provider::try_from(args.provider)?.build().await?;

    // Initialize managers / secrets
    let mut env_secrets = args.env;
    env_secrets.extend(args.env_file);
    let env_manager = EnvManager::new(env_secrets, provider.clone());

    let interactive = args.interactive.unwrap_or(!args.watch);
    let command = ShellCommand::try_from(args.cmd)?;
    let mut process = ProcessManager::new(env_manager, command, interactive, args.timeout);

    let files = SecretFileManager::new(args.files, provider)?;

    // Initial Start
    info!("resolving environment and starting process...");
    files.inject_all().await?;
    process.start().await?;

    let mut handler = ExecOrchestrator::new(process, files);

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

struct ExecOrchestrator {
    process: ProcessManager,
    files: SecretFileManager,
    process_paths: HashSet<AbsolutePath>,
}

impl ExecOrchestrator {
    pub fn new(process: ProcessManager, files: SecretFileManager) -> Self {
        let process_paths = process.paths().into_iter().collect();
        Self {
            process,
            files,
            process_paths,
        }
    }
}

#[async_trait::async_trait]
impl EventHandler for ExecOrchestrator {
    fn paths(&self) -> Vec<AbsolutePath> {
        let mut p = self.files.paths();
        p.extend(self.process.paths());
        p
    }

    async fn handle(&mut self, events: Vec<FsEvent>) -> Result<(), HandlerError> {
        let proc_events: Vec<FsEvent> = events
            .iter()
            .filter(|e| e.affects(|p| self.process_paths.contains(p)))
            .cloned()
            .collect();

        // SecretFileManager will ignore paths it does not manage.
        // so it's best to just pass all events
        self.files.handle(events).await?;

        // Handle Process Restarts
        if !proc_events.is_empty() {
            self.process.handle(proc_events).await?;
        }

        Ok(())
    }

    fn wait(&self) -> BoxFuture<'static, Result<(), HandlerError>> {
        // Lifecycle is dictated by the child process, not the files.
        self.process.wait()
    }

    async fn cleanup(&mut self) {
        self.process.cleanup().await;
    }
}
