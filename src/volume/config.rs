use super::types::MountFlags;
use crate::config::parsers::polymorphic_vec;
use crate::error::LocketError;
use crate::path::AbsolutePath;
use crate::provider::{ProviderArgs, SecretsProvider};
use crate::secrets::{
    InjectFailurePolicy, MemSize, Secret, SecretFileManager, SecretManagerConfig,
};
use crate::volume::types::DockerOptions;
use crate::write::{FileWriter, FileWriterArgs, FsMode};
use clap::Args;
use locket_derive::LayeredConfig;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Map, Value};
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
    #[serde(default, deserialize_with = "polymorphic_vec", alias = "secret")]
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

impl TryFrom<DockerOptions> for VolumeArgs {
    type Error = LocketError;

    fn try_from(opts: DockerOptions) -> Result<Self, Self::Error> {
        let json_val = expand_docker_options(opts);

        let args: VolumeArgs = serde_json::from_value(json_val).map_err(|e| {
            LocketError::Config(format!("Invalid volume configuration: {}", e).into())
        })?;

        Ok(args)
    }
}

fn expand_docker_options(opts: DockerOptions) -> Value {
    let mut root = Map::new();

    for (k, v) in opts {
        let (key, val) = if let Some((prefix, remainder)) = k.split_once('.') {
            (prefix, format!("{}={}", remainder, v))
        } else {
            (k.as_str(), v)
        };

        match root.entry(key.to_string()) {
            serde_json::map::Entry::Vacant(e) => {
                e.insert(Value::String(val));
            }
            serde_json::map::Entry::Occupied(mut e) => {
                let mut list = match e.get_mut().take() {
                    Value::Array(vec) => vec,
                    other => vec![other],
                };
                list.push(Value::String(val));
                e.insert(Value::Array(list));
            }
        }
    }

    Value::Object(root)
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
