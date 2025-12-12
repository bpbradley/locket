use crate::{
    env::EnvManager,
    logging::Logger,
    provider::Provider,
    secrets::Secret,
    signal,
    watch::{DebounceDuration, ExecError, FsWatcher, ProcessHandler},
};
use clap::Args;
use sysexits::ExitCode;
use tracing::{debug, error, info};

#[derive(Args, Debug)]
pub struct ExecArgs {
    /// Mode of operation
    #[arg(long = "mode", env = "LOCKET_EXEC_WATCH", default_value_t = false)]
    pub watch: bool,

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

    /// Secrets to be injected in environment
    #[arg(
        long,
        short = 'e',
        env = "LOCKET_EXEC_ENV",
        value_name = "KEY=VAL, KEY=@FILE or /path/to/.env",
        value_delimiter = ',',
        hide_env_values = true,
        help_heading = None,
        action = clap::ArgAction::Append,
    )]
    pub env: Vec<Secret>,

    /// Secrets provider selection
    #[command(flatten, next_help_heading = "Provider Configuration")]
    provider: Provider,

    /// Command to execute with secrets injected into environment
    /// Example: locket exec -e locket.env -- docker compose up -d
    #[arg(required = true, trailing_var_arg = true)]
    pub cmd: Vec<String>,
}

pub async fn exec(args: ExecArgs) -> ExitCode {
    if let Err(e) = args.logger.init() {
        error!(error=%e, "init logging failed");
        return ExitCode::CantCreat;
    }
    info!(
        "Starting locket v{} `exec` service ",
        env!("CARGO_PKG_VERSION")
    );
    debug!("effective config: {:#?}", args);

    // Initialize Provider
    let provider = match args.provider.build().await {
        Ok(p) => p,
        Err(e) => {
            error!(error = %e, "failed to initialize secrets provider");
            return ExitCode::Config;
        }
    };

    // Initialize EnvManager (Stateless)
    // We pass the raw secrets; resolution happens inside the handler.
    let env_manager = EnvManager::new(args.env, provider);

    // Initialize ProcessHandler
    let mut handler = ProcessHandler::new(env_manager, args.cmd.clone());

    // Initial Start
    // We must start the process at least once regardless of mode.
    info!("resolving environment and starting process...");
    if let Err(e) = handler.start().await {
        error!(error = %e, "failed to start process");
        // Distinguish between configuration errors (e.g. template missing) and IO errors
        return ExitCode::Unavailable;
    }

    // Execution Mode Branch
    if args.watch {
        let watcher = FsWatcher::new(args.debounce, handler);

        // Run the watcher loop until a shutdown signal (Ctrl+C/SIGTERM) is received
        match watcher.run(signal::recv_shutdown()).await {
            Ok(mut handler) => {
                info!("watch loop terminated gracefully");
                handler.stop().await;
                ExitCode::Ok
            }
            Err(e) => {
                error!(error = %e, "watch loop failed");
                ExitCode::Software
            }
        }
    } else {
        if let Err(e) = handler.wait().await {
            error!(error = %e, "process execution failed");
            return e.into();
        }
        ExitCode::Ok
    }
}

impl From<ExecError> for ExitCode {
    fn from(_err: ExecError) -> Self {
        ExitCode::Software
    }
}
