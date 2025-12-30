use locket::error::LocketError;
use locket::events::HandlerError;
use locket::provider::ProviderError;
use locket::secrets::SecretError;
use locket::watch::WatchError;
use std::os::unix::process::ExitStatusExt;
use std::process::{ExitCode, Termination};

#[derive(Debug)]
pub struct LocketExitCode(pub LocketError);

impl Termination for LocketExitCode {
    fn report(self) -> ExitCode {
        let code = self.exit_code();
        tracing::error!(exit_code = code, "{}", self.0);
        ExitCode::from(code)
    }
}

impl LocketExitCode {
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
                SecretError::Write(_) => sysexits::ExitCode::IoErr.into(),
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
                ProviderError::Url(_) => sysexits::ExitCode::DataErr.into(),
            },
            LocketError::ReferenceParse(_) => sysexits::ExitCode::DataErr.into(),
            LocketError::Watch(e) => match e {
                WatchError::Io(_) => sysexits::ExitCode::IoErr.into(),
                WatchError::Notify(_) => sysexits::ExitCode::Software.into(),
                WatchError::SourceMissing(_) => sysexits::ExitCode::Config.into(),
                WatchError::Disconnected => sysexits::ExitCode::Software.into(),
                WatchError::Handler(h) => Self::handler_exit_code(h),
            },
            LocketError::Handler(e) => Self::handler_exit_code(e),
            LocketError::Health(e) => match e {
                locket::health::HealthError::Unhealthy => 1,
                locket::health::HealthError::Io(_) => sysexits::ExitCode::IoErr.into(),
            },

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
            locket::process::ProcessError::InvalidCommand(_) => sysexits::ExitCode::Usage.into(),
        }
    }
}
