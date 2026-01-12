use crate::config::de::polymorphic_vec;
use crate::path::{AbsolutePath, PathMapping};
use crate::secrets::{MemSize, Secret, SecretError};
use crate::write::{FileWriter, FileWriterArgs};
use clap::{Args, ValueEnum};
use locket_derive::LayeredConfig;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub struct SecretManagerConfig {
    pub map: Vec<PathMapping>,
    pub secrets: Vec<Secret>,
    pub out: AbsolutePath,
    pub inject_failure_policy: InjectFailurePolicy,
    pub max_file_size: MemSize,
    pub writer: FileWriter,
}

impl Default for SecretManagerConfig {
    fn default() -> Self {
        SecretManagerConfig {
            map: Vec::new(),
            secrets: Vec::new(),
            #[cfg(target_os = "linux")]
            out: AbsolutePath::new("/run/secrets/locket"),
            #[cfg(target_os = "macos")]
            out: AbsolutePath::new("/private/tmp/locket"),
            #[cfg(not(any(target_os = "linux", target_os = "macos")))]
            out: AbsolutePath::new("./secrets"), // Fallback
            inject_failure_policy: InjectFailurePolicy::default(),
            max_file_size: MemSize::default(),
            writer: FileWriter::default(),
        }
    }
}

impl SecretManagerConfig {
    pub fn validate_structure(&mut self) -> Result<(), SecretError> {
        let mut sources = Vec::new();
        let mut destinations = Vec::new();

        for m in &self.map {
            sources.push(m.src());
            destinations.push(m.dst());
        }
        destinations.push(&self.out);

        // Check for feedback loops and self-destruct scenarios
        for src in &sources {
            for dst in &destinations {
                if dst.starts_with(src) {
                    return Err(SecretError::Loop {
                        src: src.to_path_buf(),
                        dst: dst.to_path_buf(),
                    });
                }
                if src.starts_with(dst) {
                    return Err(SecretError::Destructive {
                        src: src.to_path_buf(),
                        dst: dst.to_path_buf(),
                    });
                }
            }
        }
        Ok(())
    }
}

#[derive(Copy, Clone, Debug, ValueEnum, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum InjectFailurePolicy {
    /// Failures are treated as errors and will abort the process
    Error,
    /// On failure, copy the unmodified secret to destination
    #[default]
    Passthrough,
    /// On failure, ignore the secret and log a warning
    Ignore,
}

impl std::fmt::Display for InjectFailurePolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.to_possible_value()
            .expect("no values are skipped")
            .get_name()
            .fmt(f)
    }
}

#[derive(Debug, Clone, Args, Deserialize, Serialize, LayeredConfig, Default)]
#[serde(rename_all = "kebab-case")]
#[locket(try_into = "SecretManagerConfig")]
pub struct SecretManagerArgs {
    /// Mapping of source paths to destination paths.
    ///
    /// Maps sources (holding secret templates) to destination paths
    /// (where secrets are materialized) in the form `SRC:DST` or `SRC=DST`.
    ///
    /// Multiple mappings can be provided, separated by commas, or supplied
    /// multiple times as arguments.
    ///
    /// Example: `--map /templates:/run/secrets/app`
    ///
    /// **CLI Default:** No mappings
    /// {n}**Docker Default:** `/templates:/run/secrets/locket`
    #[arg(
        long,
        alias = "secret_map",
        env = "SECRET_MAP",
        value_delimiter = ',',
        hide_env_values = true
    )]
    #[serde(alias = "secret_map", default, deserialize_with = "polymorphic_vec")]
    #[locket(docs = "
        TOML syntax supports list of strings or map form:
        List form:
        map = [\"/templates:/run/secrets/app\", \"/config:/run/secrets/config\"]

        Map form:
        [map]
        source = \"/templates\"
        destination = \"/run/secrets/app\"
        [map]
        source = \"/config\"
        destination = \"/run/secrets/config\"
    ")]
    pub map: Vec<PathMapping>,

    /// Additional secret values specified as LABEL=SECRET_TEMPLATE
    ///
    /// Multiple values can be provided, separated by commas.
    /// Or supplied multiple times as arguments.
    ///
    /// Loading from file is supported via `LABEL=@/path/to/file`.
    ///
    /// Example:
    ///
    /// ```sh
    ///     --secret db_password={{op://..}}
    ///     --secret api_key={{op://..}}
    /// ```
    #[arg(
        long,
        alias = "secret",
        env = "LOCKET_SECRETS",
        value_name = "label={{template}}",
        value_delimiter = ',',
        hide_env_values = true
    )]
    #[serde(alias = "secret", deserialize_with = "polymorphic_vec", default)]
    #[locket(docs = "
        TOML syntax supports list of strings or map form:
        List form:
        secrets = [\"db_password={{..}}\", \"api_key={{..}}\"]

        Map form:
        [secrets]
        db_password = \"{{..}}\"
        api_key = \"{{..}}\"
    ")]
    pub secrets: Vec<Secret>,

    /// Directory where secret values (literals) are materialized
    #[arg(long, env = "DEFAULT_SECRET_DIR")]
    #[locket(default = SecretManagerConfig::default().out)]
    pub out: Option<AbsolutePath>,

    /// Policy for handling injection failures
    #[arg(long, env = "INJECT_POLICY", value_enum)]
    #[locket(default = InjectFailurePolicy::Passthrough)]
    pub inject_failure_policy: Option<InjectFailurePolicy>,

    /// Maximum allowable size for a template file. Files larger than this will be rejected.
    ///
    /// Supports human-friendly suffixes like K, M, G (e.g. 10M = 10 Megabytes).
    #[arg(long, env = "MAX_FILE_SIZE")]
    #[locket(default = MemSize::default())]
    pub max_file_size: Option<MemSize>,

    /// File writing permissions
    #[command(flatten)]
    #[serde(flatten)]
    pub writer: FileWriterArgs,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Deserialize, Debug)]
    struct TestWrapper {
        #[serde(deserialize_with = "crate::config::de::polymorphic_vec")]
        secrets: Vec<Secret>,
    }

    #[test]
    fn test_list_syntax_named_and_anonymous() {
        let toml_input = r#"
            secrets = [
                "name=template",
                "."
            ]
        "#;

        let parsed: TestWrapper = toml::from_str(toml_input).expect("Should parse list");
        assert_eq!(parsed.secrets.len(), 2);

        let named = &parsed.secrets[0];
        let anonymous = &parsed.secrets[1];
        if let Secret::Named {
            key: label,
            source: _value,
        } = named
        {
            assert_eq!(label.as_ref(), "name");
        } else {
            panic!("Expected first secret to be Named, but got: {:?}", named);
        }
        assert!(matches!(anonymous, Secret::Anonymous(_)));
    }

    #[test]
    fn test_map_syntax_converts_to_named() {
        let toml_input = r#"
            [secrets]
            key = "val"
            "custom" = "val"
        "#;

        let parsed: TestWrapper = toml::from_str(toml_input).expect("Should parse map");

        assert_eq!(parsed.secrets.len(), 2);

        let debug_out = format!("{:?}", parsed.secrets);
        assert!(debug_out.contains("key"));
        assert!(debug_out.contains("custom"));
    }
}
