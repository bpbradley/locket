use crate::{
    events::HandlerError, provider::ProviderError, secrets::SecretError, watch::WatchError,
};
use std::os::unix::process::ExitStatusExt;
use sysexits::ExitCode;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum LocketError {
    #[error(transparent)]
    Secret(#[from] SecretError),

    #[error(transparent)]
    Provider(#[from] ProviderError),

    #[error(transparent)]
    Watch(#[from] WatchError),

    #[error(transparent)]
    Handler(#[from] HandlerError),

    #[cfg(feature = "exec")]
    #[error(transparent)]
    Process(#[from] crate::process::ProcessError),

    #[cfg(feature = "compose")]
    #[error(transparent)]
    Compose(#[from] crate::compose::ComposeError),

    #[cfg(any(feature = "exec", feature = "compose"))]
    #[error(transparent)]
    Env(#[from] crate::env::EnvError),

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    Anyhow(#[from] anyhow::Error),
}

impl std::process::Termination for LocketError {
    fn report(self) -> std::process::ExitCode {
        let code = self.exit_code();
        tracing::error!(exit_code = code, "{}", self);
        std::process::ExitCode::from(code)
    }
}

impl LocketError {
    pub fn exit_code(&self) -> u8 {
        match self {
            LocketError::Secret(e) => match e {
                SecretError::Io(_) => ExitCode::IoErr.into(),
                SecretError::Provider(_) => ExitCode::Config.into(),
                SecretError::Task(_) => ExitCode::Software.into(),
                SecretError::SourceTooLarge { .. } => ExitCode::DataErr.into(),
                SecretError::Collision { .. } => ExitCode::Usage.into(), // Or Config?
                SecretError::StructureConflict { .. } => ExitCode::Usage.into(),
                SecretError::SourceMissing(_) => ExitCode::NoInput.into(),
                SecretError::Loop { .. } => ExitCode::Usage.into(),
                SecretError::Destructive { .. } => ExitCode::Usage.into(),
                SecretError::NoParent(_) => ExitCode::IoErr.into(),
                SecretError::Parse(_) => ExitCode::DataErr.into(),
            },
            LocketError::Provider(e) => match e {
                ProviderError::Network(_) => ExitCode::Unavailable.into(),
                ProviderError::NotFound(_) => ExitCode::NoInput.into(),
                ProviderError::Unauthorized(_) => ExitCode::NoPerm.into(),
                ProviderError::RateLimit => ExitCode::TempFail.into(),
                ProviderError::Other(_) => ExitCode::Software.into(),
                ProviderError::InvalidConfig(_) => ExitCode::Config.into(),
                ProviderError::Io(_) => ExitCode::IoErr.into(),
                ProviderError::Exec { .. } => ExitCode::Unavailable.into(),
            },
            LocketError::Watch(e) => match e {
                WatchError::Io(_) => ExitCode::IoErr.into(),
                WatchError::Notify(_) => ExitCode::Software.into(),
                WatchError::SourceMissing(_) => ExitCode::Config.into(),
                WatchError::Disconnected => ExitCode::Software.into(),
                WatchError::Handler(h) => Self::handler_exit_code(h),
            },
            LocketError::Handler(e) => Self::handler_exit_code(e),

            // Feature specific
            #[cfg(feature = "exec")]
            LocketError::Process(e) => Self::process_exit_code(e),

            #[cfg(feature = "compose")]
            LocketError::Compose(e) => match e {
                crate::compose::ComposeError::Io(_) => ExitCode::IoErr.into(),
                crate::compose::ComposeError::Provider(_) => ExitCode::Unavailable.into(),
                crate::compose::ComposeError::Secret(_) => ExitCode::Config.into(),
                crate::compose::ComposeError::Configuration(_) => ExitCode::Config.into(),
                crate::compose::ComposeError::Argument(_) => ExitCode::Usage.into(),
            },

            #[cfg(any(feature = "exec", feature = "compose"))]
            LocketError::Env(e) => match e {
                crate::env::EnvError::Io(_) => ExitCode::IoErr.into(),
                crate::env::EnvError::Secret(_) => ExitCode::Config.into(),
                crate::env::EnvError::Provider(_) => ExitCode::Unavailable.into(),
                crate::env::EnvError::Parse(_) => ExitCode::DataErr.into(),
                crate::env::EnvError::Join(_) => ExitCode::Software.into(),
            },

            // Generic
            LocketError::Io(_) => ExitCode::IoErr.into(),
            LocketError::Anyhow(_) => ExitCode::Software.into(),
        }
    }

    fn handler_exit_code(e: &HandlerError) -> u8 {
        match e {
            HandlerError::Io(_) => ExitCode::IoErr.into(),
            HandlerError::Secret(_) => ExitCode::Software.into(),
            HandlerError::Provider(_) => ExitCode::Unavailable.into(),
            HandlerError::Exited(status) => {
                if let Some(code) = status.code() {
                    code as u8
                } else if let Some(signal) = status.signal() {
                    (128 + signal) as u8
                } else {
                    ExitCode::Unavailable.into()
                }
            }
            HandlerError::Signaled => 128 + 15, // SIGTERM
            HandlerError::Interrupted => ExitCode::Ok.into(),
            #[cfg(any(feature = "exec", feature = "compose"))]
            HandlerError::Process(e) => Self::process_exit_code(e),
            #[cfg(any(feature = "exec", feature = "compose"))]
            HandlerError::Env(_) => ExitCode::Config.into(),
        }
    }

    #[cfg(feature = "exec")]
    fn process_exit_code(e: &crate::process::ProcessError) -> u8 {
        match e {
            crate::process::ProcessError::Env(_) => ExitCode::Config.into(),
            crate::process::ProcessError::Io(e) => match e.kind() {
                std::io::ErrorKind::NotFound => 127,
                std::io::ErrorKind::PermissionDenied => 126,
                _ => ExitCode::IoErr.into(),
            },
            crate::process::ProcessError::Exited(status) => {
                if let Some(code) = status.code() {
                    code as u8
                } else if let Some(signal) = status.signal() {
                    (128 + signal) as u8
                } else {
                    ExitCode::Unavailable.into()
                }
            }
            crate::process::ProcessError::Signaled => 128 + 15, // SIGTERM
        }
    }
}
