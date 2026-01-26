use super::types::MountFlags;
use crate::config::parsers::polymorphic_vec;
use crate::error::LocketError;
use crate::path::AbsolutePath;
use crate::provider::{ProviderArgs, SecretsProvider};
use crate::secrets::{
    InjectFailurePolicy, MemSize, Secret, SecretFileManager, SecretManagerConfig,
};
use crate::write::{FileWriter, FileWriterArgs, FsMode};
use clap::Args;
use locket_derive::LayeredConfig;
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Default, Debug, Clone, Serialize, Deserialize)]
pub struct VolumeSpec {
    pub secrets: Vec<Secret>,
    pub watch: bool,
    pub inject_failure_policy: InjectFailurePolicy,
    pub max_file_size: MemSize,
    pub writer: FileWriter,
    pub mount: MountConfig,
    pub provider: ProviderArgs,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MountConfig {
    pub size: MemSize,
    pub mode: FsMode,
    pub flags: MountFlags,
}

impl Default for MountConfig {
    fn default() -> Self {
        MountConfig {
            size: MemSize::from_mb(10),
            mode: FsMode::new(0o700),
            flags: MountFlags::default(),
        }
    }
}

#[derive(Args, Debug, Clone, Default, Serialize, Deserialize, LayeredConfig)]
#[serde(rename_all = "kebab-case")]
#[locket(try_into = "MountConfig")]
pub struct MountOptions {
    /// Default size of the in-memory filesystem
    #[arg(long, env = "LOCKET_VOLUME_DEFAULT_MOUNT_SIZE")]
    #[locket(default = MountConfig::default().size)]
    pub size: Option<MemSize>,
    /// Default file mode for the mounted filesystem
    #[arg(long, env = "LOCKET_VOLUME_DEFAULT_MOUNT_MODE")]
    #[locket(default = MountConfig::default().mode)]
    pub mode: Option<FsMode>,
    /// Default mount flags for the in-memory filesystem
    #[arg(long, env = "LOCKET_VOLUME_DEFAULT_MOUNT_FLAGS")]
    #[locket(default = MountConfig::default().flags)]
    pub flags: Option<MountFlags>,
}

#[derive(Args, Debug, Clone, Default, Serialize, Deserialize, LayeredConfig)]
#[serde(rename_all = "kebab-case")]
#[locket(try_into = "VolumeSpec")]
pub struct VolumeArgs {
    /// Default secrets to mount into the volume
    ///
    /// These will typically be specified in driver_opts for volume.
    /// However, default secrets can be provided via CLI/ENV which would
    /// be available to all volumes by default.
    #[arg(
        long,
        alias = "secret",
        env = "LOCKET_VOLUME_DEFAULT_SECRETS",
        value_name = "label={{template}} or /path/to/template",
        value_delimiter = ',',
        hide_env_values = true
    )]
    #[serde(default, deserialize_with = "polymorphic_vec")]
    pub secrets: Vec<Secret>,

    /// Default behavior for file watching.
    ///
    /// If set to true, the volume will watch for changes in the secrets
    /// and update the files accordingly.
    #[arg(
        long,
        env = "LOCKET_VOLUME_DEFAULT_WATCH",
        num_args = 0..=1,
        require_equals = true
    )]
    #[serde(default, deserialize_with = "parse_bool_opt")]
    #[locket(default = false)]
    pub watch: Option<bool>,

    /// Default policy for handling failures when errors are encountered
    #[arg(long, env = "LOCKET_VOLUME_DEFAULT_INJECT_POLICY", value_enum)]
    #[locket(default = InjectFailurePolicy::Passthrough)]
    pub inject_failure_policy: Option<InjectFailurePolicy>,

    /// Default maximum size of individual secret files
    #[arg(long, env = "LOCKET_VOLUME_DEFAULT_MAX_FILE_SIZE")]
    #[locket(default = MemSize::from_mb(10))]
    pub max_file_size: Option<MemSize>,

    #[serde(flatten)]
    #[clap(flatten)]
    pub writer: FileWriterArgs,

    #[serde(flatten)]
    #[clap(flatten)]
    pub mount: MountOptions,

    #[serde(flatten)]
    #[clap(flatten)]
    pub provider: ProviderArgs,
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
