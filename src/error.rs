use crate::{
    events::HandlerError, provider::ProviderError, secrets::SecretError, watch::WatchError,
};
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
    Logging(#[from] crate::logging::LoggingError),
}
