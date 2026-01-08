use crate::error::LocketError;
use crate::path::AbsolutePath;
use clap::Args;
use serde::de::DeserializeOwned;
use std::collections::HashMap;
use std::path::Path;
use thiserror::Error;

#[cfg(feature = "exec")]
pub mod exec;
pub mod inject;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("failed to load configuration file: {0}")]
    Io(#[from] std::io::Error),

    #[error("failed to parse TOML configuration: {0}")]
    Parse(#[from] toml::de::Error),

    #[error("{0}")]
    Validation(String),

    #[cfg(feature = "exec")]
    #[error(transparent)]
    Process(#[from] crate::process::ProcessError),
}

/// Trait for merging two partial structs.
pub trait Overlay {
    /// self is the base layer, over is the top layer.
    fn overlay(self, over: Self) -> Self;
}

// If top layer exists, use it. Otherwise keep base.
impl<T> Overlay for Option<T> {
    fn overlay(self, over: Self) -> Self {
        over.or(self)
    }
}

impl<T> Overlay for Vec<T> {
    fn overlay(self, over: Self) -> Self {
        if over.is_empty() { self } else { over }
    }
}

#[derive(Args, Debug, Clone)]
pub struct LayeredArgs<T: Args> {
    /// Path to configuration file
    #[arg(long, env = "LOCKET_CONFIG")]
    pub config: Option<AbsolutePath>,

    #[command(flatten)]
    pub inner: T,
}

impl<T> LayeredArgs<T>
where
    T: Args,
{
    pub fn load<C>(self) -> Result<C, crate::error::LocketError>
    where
        T: Layered<C>,
    {
        self.inner.resolve(self.config.as_deref())
    }
}

pub trait Layered<C>: Overlay + DeserializeOwned + Default + Sized {
    fn resolve(self, config_path: Option<&Path>) -> Result<C, LocketError>;
}

impl<T, C> Layered<C> for T
where
    T: Overlay + DeserializeOwned + Default,
    T: TryInto<C>,
    <T as TryInto<C>>::Error: Into<LocketError>,
{
    fn resolve(self, config_path: Option<&Path>) -> Result<C, LocketError> {
        let base = if let Some(path) = config_path {
            if path.exists() {
                let content = std::fs::read_to_string(path).map_err(ConfigError::Io)?;

                toml::from_str::<Self>(&content).map_err(ConfigError::Parse)?
            } else {
                Self::default()
            }
        } else {
            Self::default()
        };

        let merged = base.overlay(self);

        merged.try_into().map_err(Into::into)
    }
}

/// Trait for applying configured default values to optional fields.
pub trait ApplyDefaults {
    fn apply_defaults(self) -> Self;
}

/// Trait to expose defaults defined in #[locket(default = ...)] for documentation generation.
pub trait LocketDocDefaults {
    fn register_defaults(map: &mut HashMap<String, String>);

    /// Helper to get all defaults as a map
    fn get_defaults() -> HashMap<String, String> {
        let mut map = HashMap::new();
        Self::register_defaults(&mut map);
        map
    }
}
