//! Main entry point for the secret-sidecar binary.
//!
//! This binary provides the `locket` command-line interface,
//! and otherwise serves as a thin dispatch layer for `locket`
use clap::Parser;
use locket::cmd;
use locket::cmd::{Cli, Command};
use locket::error::LocketError;
use locket::events::HandlerError;
use locket::provider::ProviderError;
use locket::secrets::SecretError;
use locket::watch::WatchError;
use std::os::unix::process::ExitStatusExt;
use std::process::{ExitCode, Termination};

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    let result: Result<(), LocketError> = match cli.cmd {
        Command::Run(args) => cmd::run(*args).await,
        #[cfg(feature = "exec")]
        Command::Exec(args) => cmd::exec(*args).await,
        Command::Healthcheck(args) => cmd::healthcheck(args),
        #[cfg(feature = "compose")]
        Command::Compose(args) => cmd::compose(*args).await,
        #[cfg(feature = "compose")]
        Command::DockerCliPluginMetadata => {
            let metadata = serde_json::json!({
                "SchemaVersion": "0.1.0",
                "Vendor": "Brian Bradley",
                "Version": env!("CARGO_PKG_VERSION"),
                "ShortDescription": "Secret management for Docker Compose",
                "URL": "https://github.com/bpbradley/locket"
            });
            println!("{}", metadata);
            Ok(())
        }
    };
    match result {
        Ok(_) => ExitCode::SUCCESS,
        Err(e) => AppError(e).report(),
    }
}

#[derive(Debug)]
struct AppError(LocketError);

impl Termination for AppError {
    fn report(self) -> ExitCode {
        let code = self.exit_code();
        tracing::error!(exit_code = code, "{}", self.0);
        ExitCode::from(code)
    }
}

impl AppError {
    fn exit_code(&self) -> u8 {
        match &self.0 {
            LocketError::Secret(e) => match e {
                SecretError::Io(_) => sysexits::ExitCode::IoErr.into(),
                SecretError::Provider(_) => sysexits::ExitCode::Config.into(),
                SecretError::Task(_) => sysexits::ExitCode::Software.into(),
                SecretError::SourceTooLarge { .. } => sysexits::ExitCode::DataErr.into(),
                SecretError::Collision { .. } => sysexits::ExitCode::Usage.into(),
                SecretError::StructureConflict { .. } => sysexits::ExitCode::Usage.into(),
                SecretError::SourceMissing(_) => sysexits::ExitCode::NoInput.into(),
                SecretError::Loop { .. } => sysexits::ExitCode::Usage.into(),
                SecretError::Destructive { .. } => sysexits::ExitCode::Usage.into(),
                SecretError::NoParent(_) => sysexits::ExitCode::IoErr.into(),
                SecretError::Parse(_) => sysexits::ExitCode::DataErr.into(),
            },
            LocketError::Provider(e) => match e {
                ProviderError::Network(_) => sysexits::ExitCode::Unavailable.into(),
                ProviderError::NotFound(_) => sysexits::ExitCode::NoInput.into(),
                ProviderError::Unauthorized(_) => sysexits::ExitCode::NoPerm.into(),
                ProviderError::RateLimit => sysexits::ExitCode::TempFail.into(),
                ProviderError::Other(_) => sysexits::ExitCode::Software.into(),
                ProviderError::InvalidConfig(_) => sysexits::ExitCode::Config.into(),
                ProviderError::Io(_) => sysexits::ExitCode::IoErr.into(),
                ProviderError::Exec { .. } => sysexits::ExitCode::Unavailable.into(),
            },
            LocketError::Watch(e) => match e {
                WatchError::Io(_) => sysexits::ExitCode::IoErr.into(),
                WatchError::Notify(_) => sysexits::ExitCode::Software.into(),
                WatchError::SourceMissing(_) => sysexits::ExitCode::Config.into(),
                WatchError::Disconnected => sysexits::ExitCode::Software.into(),
                WatchError::Handler(h) => Self::handler_exit_code(h),
            },
            LocketError::Handler(e) => Self::handler_exit_code(e),

            #[cfg(feature = "exec")]
            LocketError::Process(e) => Self::process_exit_code(e),

            #[cfg(feature = "compose")]
            LocketError::Compose(e) => match e {
                locket::compose::ComposeError::Io(_) => sysexits::ExitCode::IoErr.into(),
                locket::compose::ComposeError::Provider(_) => {
                    sysexits::ExitCode::Unavailable.into()
                }
                locket::compose::ComposeError::Secret(_) => sysexits::ExitCode::Config.into(),
                locket::compose::ComposeError::Configuration(_) => {
                    sysexits::ExitCode::Config.into()
                }
                locket::compose::ComposeError::Argument(_) => sysexits::ExitCode::Usage.into(),
                locket::compose::ComposeError::Metadata(_) => sysexits::ExitCode::Software.into(),
            },

            #[cfg(any(feature = "exec", feature = "compose"))]
            LocketError::Env(e) => match e {
                locket::env::EnvError::Io(_) => sysexits::ExitCode::IoErr.into(),
                locket::env::EnvError::Secret(_) => sysexits::ExitCode::Config.into(),
                locket::env::EnvError::Provider(_) => sysexits::ExitCode::Unavailable.into(),
                locket::env::EnvError::Parse(_) => sysexits::ExitCode::DataErr.into(),
                locket::env::EnvError::Join(_) => sysexits::ExitCode::Software.into(),
            },

            LocketError::Io(_) => sysexits::ExitCode::IoErr.into(),
            LocketError::Logging(_) => sysexits::ExitCode::Config.into(),
        }
    }

    fn handler_exit_code(e: &HandlerError) -> u8 {
        match e {
            HandlerError::Io(_) => sysexits::ExitCode::IoErr.into(),
            HandlerError::Secret(_) => sysexits::ExitCode::Software.into(),
            HandlerError::Provider(_) => sysexits::ExitCode::Unavailable.into(),
            HandlerError::Exited(status) => {
                if let Some(code) = status.code() {
                    code as u8
                } else if let Some(signal) = status.signal() {
                    (128 + signal) as u8
                } else {
                    sysexits::ExitCode::Unavailable.into()
                }
            }
            HandlerError::Signaled => 128 + 15,
            HandlerError::Interrupted => sysexits::ExitCode::Ok.into(),
            #[cfg(any(feature = "exec", feature = "compose"))]
            HandlerError::Process(e) => Self::process_exit_code(e),
            #[cfg(any(feature = "exec", feature = "compose"))]
            HandlerError::Env(_) => sysexits::ExitCode::Config.into(),
        }
    }

    #[cfg(feature = "exec")]
    fn process_exit_code(e: &locket::process::ProcessError) -> u8 {
        match e {
            locket::process::ProcessError::Env(_) => sysexits::ExitCode::Config.into(),
            locket::process::ProcessError::Io(e) => match e.kind() {
                std::io::ErrorKind::NotFound => 127,
                std::io::ErrorKind::PermissionDenied => 126,
                _ => sysexits::ExitCode::IoErr.into(),
            },
            locket::process::ProcessError::Exited(status) => {
                if let Some(code) = status.code() {
                    code as u8
                } else if let Some(signal) = status.signal() {
                    (128 + signal) as u8
                } else {
                    sysexits::ExitCode::Unavailable.into()
                }
            }
            locket::process::ProcessError::Signaled => 128 + 15,
        }
    }
}
