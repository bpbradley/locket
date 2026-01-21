use crate::config::parsers::polymorphic_vec;
use crate::error::LocketError;
use crate::path::AbsolutePath;
use crate::provider::SecretsProvider;
use crate::secrets::{
    InjectFailurePolicy, MemSize, Secret, SecretFileManager, SecretManagerConfig,
};
use crate::write::{FileWriter, FileWriterArgs, FsMode};
use locket_derive::LayeredConfig;
use nix::mount::MsFlags;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;
use std::str::FromStr;
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VolumeSpec {
    pub secrets: Vec<Secret>,
    pub watch: bool,
    pub inject_failure_policy: InjectFailurePolicy,
    pub max_file_size: MemSize,
    pub writer: FileWriter,
    pub mount: MountConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MountConfig {
    pub size: MemSize,
    pub mode: FsMode,
    pub flags: MountFlags,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, LayeredConfig)]
#[serde(rename_all = "kebab-case")]
#[locket(try_into = "MountConfig")]
pub struct MountOptions {
    #[locket(default = MemSize::from_mb(10))]
    pub size: Option<MemSize>,
    #[locket(default = FsMode::new(0o700))]
    pub mode: Option<FsMode>,
    #[locket(default = MountFlags::default())]
    pub flags: Option<MountFlags>,
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

    #[locket(default = MemSize::from_mb(10))]
    pub max_file_size: Option<MemSize>,

    #[serde(flatten)]
    #[locket(allow_mismatched_flatten)]
    pub writer: FileWriterArgs,

    #[serde(flatten)]
    #[locket(allow_mismatched_flatten)]
    pub mount: MountOptions,
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
#[derive(Debug, Clone)]
pub struct MountFlags(MsFlags);

impl Default for MountFlags {
    fn default() -> Self {
        Self(MsFlags::MS_NOEXEC | MsFlags::MS_NOSUID | MsFlags::MS_NODEV)
    }
}

impl From<MountFlags> for MsFlags {
    fn from(f: MountFlags) -> Self {
        f.0
    }
}

impl FromStr for MountFlags {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut flags = MsFlags::empty();

        for part in s.split(',') {
            match part.trim() {
                // Restrictions
                "ro" => flags |= MsFlags::MS_RDONLY,
                "noexec" => flags |= MsFlags::MS_NOEXEC,
                "nosuid" => flags |= MsFlags::MS_NOSUID,
                "nodev" => flags |= MsFlags::MS_NODEV,
                "noatime" => flags |= MsFlags::MS_NOATIME,

                // Permissions
                "rw" => flags.remove(MsFlags::MS_RDONLY),
                "exec" => flags.remove(MsFlags::MS_NOEXEC),
                "suid" => flags.remove(MsFlags::MS_NOSUID),
                "dev" => flags.remove(MsFlags::MS_NODEV),

                "defaults" => {}

                "" => continue,
                unknown => return Err(format!("Unknown mount flag: '{}'", unknown)),
            }
        }
        Ok(Self(flags))
    }
}

impl fmt::Display for MountFlags {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut parts = Vec::new();
        if self.0.contains(MsFlags::MS_RDONLY) {
            parts.push("ro");
        } else {
            parts.push("rw");
        }
        if self.0.contains(MsFlags::MS_NOEXEC) {
            parts.push("noexec");
        } else {
            parts.push("exec");
        }
        if self.0.contains(MsFlags::MS_NOSUID) {
            parts.push("nosuid");
        } else {
            parts.push("suid");
        }
        if self.0.contains(MsFlags::MS_NODEV) {
            parts.push("nodev");
        } else {
            parts.push("dev");
        }
        write!(f, "{}", parts.join(","))
    }
}

impl<'de> Deserialize<'de> for MountFlags {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

impl Serialize for MountFlags {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}
