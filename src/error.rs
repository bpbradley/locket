use crate::{
    config::ConfigError,
    events::HandlerError,
    health::HealthError,
    logging::LoggingError,
    provider::{ProviderError, ReferenceParseError},
    secrets::SecretError,
    watch::WatchError,
};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum LocketError {
    #[error(transparent)]
    Secret(#[from] SecretError),

    #[error(transparent)]
    Provider(#[from] ProviderError),

    #[error(transparent)]
    ReferenceParse(#[from] ReferenceParseError),

    #[error(transparent)]
    Watch(#[from] WatchError),

    #[error(transparent)]
    Handler(#[from] HandlerError),

    #[error(transparent)]
    Health(#[from] HealthError),

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
    Logging(#[from] LoggingError),

    #[error(transparent)]
    Config(#[from] ConfigError),
}

#[cfg(feature = "compose")]
impl From<crate::compose::MetadataError> for LocketError {
    fn from(e: crate::compose::MetadataError) -> Self {
        LocketError::Compose(e.into())
    }
}
