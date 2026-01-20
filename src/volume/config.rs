use crate::config::parsers::polymorphic_vec;
use crate::error::LocketError;
use crate::path::AbsolutePath;
use crate::provider::SecretsProvider;
use crate::secrets::{
    InjectFailurePolicy, MemSize, Secret, SecretFileManager, SecretManagerConfig,
};
use crate::write::{FileWriter, FileWriterArgs};
use locket_derive::LayeredConfig;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VolumeSpec {
    pub secrets: Vec<Secret>,
    pub watch: bool,
    pub inject_failure_policy: InjectFailurePolicy,
    pub max_file_size: MemSize,
    pub writer: FileWriter,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, LayeredConfig)]
#[serde(rename_all = "kebab-case")]
#[locket(try_into = "VolumeSpec")]
pub struct VolumeArgs {
    #[serde(default, deserialize_with = "polymorphic_vec")]
    pub secrets: Vec<Secret>,

    #[serde(default, deserialize_with = "parse_bool_opt")]
    #[locket(default = false)]
    pub watch: Option<bool>,

    #[locket(default = InjectFailurePolicy::Passthrough)]
    pub inject_failure_policy: Option<InjectFailurePolicy>,

    #[locket(default = MemSize::default())]
    pub max_file_size: Option<MemSize>,

    #[serde(flatten)]
    #[locket(allow_mismatched_flatten)]
    pub writer: FileWriterArgs,
}

impl VolumeSpec {
    pub fn into_manager(
        self,
        mountpoint: AbsolutePath,
        provider: Arc<dyn SecretsProvider>,
    ) -> Result<SecretFileManager, LocketError> {
        let config = SecretManagerConfig::default()
            .with_secrets(self.secrets)
            .with_writer(self.writer)
            .with_outdir(mountpoint);

        SecretFileManager::new(config, provider).map_err(LocketError::from)
    }
}

//TODO: Need to refactor how I am handling configs as it is messy at this point
impl TryFrom<HashMap<String, String>> for VolumeArgs {
    type Error = LocketError;

    fn try_from(map: HashMap<String, String>) -> Result<Self, Self::Error> {
        let val = serde_json::to_value(&map).map_err(|e| LocketError::Config(e.into()))?;

        let mut args: VolumeArgs = serde_json::from_value(val)
            .map_err(|e| LocketError::Config(format!("Invalid options: {}", e).into()))?;

        for (k, v) in map {
            if k == "secret" || k.starts_with("secret.") {
                let s = if k == "secret" {
                    v.parse()?
                } else {
                    let name = k.strip_prefix("secret.").unwrap();
                    format!("{}={}", name, v).parse()?
                };
                args.secrets.push(s);
            }
        }

        Ok(args)
    }
}

use serde::Deserializer;

pub fn parse_bool_opt<'de, D>(deserializer: D) -> Result<Option<bool>, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum BoolOrString {
        Bool(bool),
        String(String),
    }

    let v: Option<BoolOrString> = Option::deserialize(deserializer)?;

    match v {
        Some(BoolOrString::Bool(b)) => Ok(Some(b)),
        Some(BoolOrString::String(s)) => match s.to_lowercase().as_str() {
            "true" | "1" | "yes" | "on" => Ok(Some(true)),
            "false" | "0" | "no" | "off" => Ok(Some(false)),
            _ => Err(serde::de::Error::custom(format!(
                "Expected boolean (true/false, 1/0, yes/no), got '{}'",
                s
            ))),
        },
        None => Ok(None),
    }
}
