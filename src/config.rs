use crate::error::LocketError;
use crate::path::AbsolutePath;
use clap::Args;
use serde::de::DeserializeOwned;
use std::path::Path;
use thiserror::Error;

#[cfg(feature = "exec")]
pub mod exec;
pub mod inject;
pub mod utils;

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

/// Trait to expose defaults defined in #[locket(default = ...)] for documentation.
#[cfg(feature = "locket-docs")]
pub trait LocketDocDefaults {
    fn register_defaults(map: &mut std::collections::HashMap<String, String>);

    /// Helper to get all defaults as a map
    fn get_defaults() -> std::collections::HashMap<String, String> {
        let mut map = std::collections::HashMap::new();
        Self::register_defaults(&mut map);
        map
    }
}

/// Trait to expose the structural keys of a configuration struct for documentation.
#[cfg(feature = "locket-docs")]
pub trait ConfigStructure {
    fn get_structure() -> Vec<(String, Option<String>)>;
}

#[cfg(test)]
mod tests {
    use crate::config::{ApplyDefaults, LayeredArgs, Overlay};
    use crate::path::AbsolutePath;
    use clap::{Args, Parser};
    use locket_derive::LayeredConfig;
    use serde::{Deserialize, Serialize};
    use std::io::Write;

    #[derive(Args, Debug, Clone, Default, Deserialize, Serialize, LayeredConfig, PartialEq)]
    #[locket(try_into = "TestConfig")]
    struct TestArgs {
        #[arg(long)]
        #[locket(default = TestConfig::default().name)]
        pub name: Option<String>,

        #[arg(long)]
        #[locket(default = TestConfig::default().port)]
        pub port: Option<u16>,
    }

    struct TestConfig {
        pub name: String,
        pub port: u16,
    }

    impl Default for TestConfig {
        fn default() -> Self {
            Self {
                name: "base".into(),
                port: 8080,
            }
        }
    }

    #[derive(Parser, Debug)]
    struct TestCli {
        #[command(flatten)]
        args: LayeredArgs<TestArgs>,
    }

    #[test]
    fn test_overlay_precedence() {
        let base = TestArgs {
            name: Some("base_name".into()),
            port: Some(1000),
        };
        let top = TestArgs {
            name: Some("top_name".into()),
            port: None,
        };

        let result = base.overlay(top);

        assert_eq!(result.name.unwrap(), "top_name");
        assert_eq!(result.port.unwrap(), 1000);
    }

    #[test]
    fn test_layered_precedence() {
        let defaults = TestArgs::default().apply_defaults();
        assert_eq!(defaults.name.as_deref(), Some("base"));

        let config_file = TestArgs {
            name: Some("config_file_name".into()),
            port: Some(9000),
        };

        let after_file = defaults.overlay(config_file.clone());
        assert_eq!(after_file.name.as_deref(), Some("config_file_name"));

        let cli_args = TestArgs {
            name: Some("cli_override".into()),
            port: None,
        };
        let final_cfg = after_file.clone().overlay(cli_args);
        assert_eq!(final_cfg.name.as_deref(), Some("cli_override"));
        assert_eq!(final_cfg.port, Some(9000)); // Kept config file value

        let empty_cli = TestArgs::default();
        let final_cfg_empty = after_file.overlay(empty_cli);
        assert_eq!(final_cfg_empty.name.as_deref(), Some("config_file_name"));
    }

    #[test]
    fn test_file_backed_loading() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        write!(
            tmp,
            r#"
            name = "from_toml"
            port = 5555
        "#
        )
        .unwrap();

        let config_path = AbsolutePath::new(tmp.path());

        let args = TestArgs {
            name: None,
            port: Some(1111),
        };

        let config = LayeredArgs {
            config: Some(config_path),
            inner: args,
        };

        let resolved: TestConfig = config.load().unwrap();

        assert_eq!(resolved.name, "from_toml");
        assert_eq!(resolved.port, 1111);
    }

    #[test]
    fn test_cli_parsing_and_layering() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        write!(
            tmp,
            r#"
                name = "file_name"
                port = 5555
            "#
        )
        .unwrap();
        let config_path = tmp.path().to_str().unwrap();

        let cli = TestCli::try_parse_from(["test_bin", "--config", config_path, "--port", "1111"])
            .unwrap();

        let resolved: TestConfig = cli.args.load().unwrap();

        assert_eq!(resolved.name, "file_name"); // From File
        assert_eq!(resolved.port, 1111); // From CLI (Override)
    }
}
