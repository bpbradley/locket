use super::{config::MountConfig, error::PluginError};
use crate::error::LocketError;
use crate::path::AbsolutePath;
use crate::write::FsOwner;
use nix::errno::Errno;
use nix::mount::{MntFlags, MsFlags, mount, umount2};
use nix::unistd::chown;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;
use std::ops::Deref;
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::str::FromStr;
use tracing::{info, warn};

pub enum VolumeType {
    Tmpfs,
}

impl VolumeType {
    pub fn as_str(&self) -> &'static str {
        match self {
            VolumeType::Tmpfs => "tmpfs",
        }
    }
}

#[derive(Debug, Clone)]
pub struct VolumeMount {
    target: AbsolutePath,
    config: MountConfig,
    owner: Option<FsOwner>,
}

impl VolumeMount {
    pub fn new(target: AbsolutePath, config: MountConfig, owner: Option<FsOwner>) -> Self {
        Self {
            target,
            config,
            owner,
        }
    }

    pub fn path(&self) -> &Path {
        &self.target
    }

    pub async fn mount(&self) -> Result<(), PluginError> {
        if !tokio::fs::try_exists(&self.target).await.unwrap_or(false) {
            tokio::fs::create_dir_all(&self.target)
                .await
                .map_err(LocketError::Io)?;
        }

        let target = self.target.clone();
        let flags: MsFlags = self.config.flags.clone().into();
        let data = format!("size={},mode={}", self.config.size, self.config.mode);

        tokio::task::spawn_blocking(move || {
            mount(
                Some(VolumeType::Tmpfs.as_str()),
                target.as_path(),
                Some(VolumeType::Tmpfs.as_str()),
                flags,
                Some(data.as_str()),
            )
        })
        .await
        .map_err(|e| PluginError::Internal(format!("Join error: {}", e)))?
        .map_err(|e| PluginError::Internal(format!("Mount failed: {}", e)))?;

        if let Some(owner) = self.owner {
            let target = self.target.clone();
            let (u, g) = owner.as_nix();

            tokio::task::spawn_blocking(move || chown(target.as_path(), Some(u), Some(g)))
                .await
                .map_err(|_| PluginError::Internal("Join error".into()))?
                .map_err(|e| PluginError::Internal(format!("Chown failed: {}", e)))?;
        }

        Ok(())
    }

    pub async fn unmount(&self) -> Result<(), PluginError> {
        let target = self.target.clone();

        // It is possible for the target to be mounted multiple times.
        // And then each mount needs to be successively unwound and unmounted.
        // This generally should not happen, but just in case
        // we will simply unmount repeatedly until the mount point is empty.
        tokio::task::spawn_blocking(move || {
            let mut attempts = 0;
            loop {
                match umount2(target.as_path(), MntFlags::empty()) {
                    Ok(_) => {
                        attempts += 1;
                    }
                    Err(Errno::EINVAL) => {
                        if attempts > 0 {
                            info!("Volume unmounted with {} layers.", attempts);
                        }
                        break;
                    }
                    Err(e) => {
                        return Err(e);
                    }
                }
            }
            Ok(())
        })
        .await
        .map_err(|_| PluginError::Internal("Join error".into()))?
        .map_err(|e| PluginError::Internal(format!("Unmount failed: {}", e)))?;

        if tokio::fs::try_exists(&self.target).await.unwrap_or(false)
            && let Err(e) = tokio::fs::remove_dir(&self.target).await
        {
            warn!("Failed to remove directory {:?}: {}", self.target, e);
        }

        Ok(())
    }

    pub async fn is_mounted(&self) -> bool {
        let path = self.target.clone();

        tokio::task::spawn_blocking(move || {
            let path = path.as_path();

            let self_meta = match std::fs::metadata(path) {
                Ok(m) => m,
                Err(_) => return false, // If path doesn't exist, it can't be mounted
            };

            let parent = match path.parent() {
                Some(p) => p,
                None => return true,
            };

            let parent_meta = match std::fs::metadata(parent) {
                Ok(m) => m,
                Err(_) => return false, // Parent missing (shouldn't happen if child exists)
            };

            // On tmpfs mounts, the device id will be different
            // this will not work in the future if bind mounts are supported
            self_meta.dev() != parent_meta.dev()
        })
        .await
        .unwrap_or(false)
    }
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DockerOptions(HashMap<String, String>);

impl DockerOptions {
    pub fn new() -> Self {
        Self(HashMap::new())
    }

    pub fn insert<S: Into<String>>(&mut self, key: S, value: S) {
        self.0.insert(key.into(), value.into());
    }

    pub fn get<S: AsRef<str>>(&self, key: S) -> Option<&String> {
        self.0.get(key.as_ref())
    }

    pub fn as_map(&self) -> &HashMap<String, String> {
        &self.0
    }

    pub fn into_inner(self) -> HashMap<String, String> {
        self.0
    }
}

impl From<HashMap<String, String>> for DockerOptions {
    fn from(map: HashMap<String, String>) -> Self {
        Self(map)
    }
}

impl<'a> IntoIterator for &'a DockerOptions {
    type Item = (&'a String, &'a String);
    type IntoIter = std::collections::hash_map::Iter<'a, String, String>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

impl IntoIterator for DockerOptions {
    type Item = (String, String);
    type IntoIter = std::collections::hash_map::IntoIter<String, String>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

impl FromIterator<(String, String)> for DockerOptions {
    fn from_iter<T: IntoIterator<Item = (String, String)>>(iter: T) -> Self {
        Self(iter.into_iter().collect())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String")]
pub struct VolumeName(String);

impl VolumeName {
    pub fn new<S: Into<String>>(name: S) -> Result<Self, LocketError> {
        let s = name.into();
        Self::validate(&s)?;
        Ok(Self(s))
    }

    fn validate(s: &str) -> Result<(), LocketError> {
        if s.is_empty() {
            return Err(LocketError::Validation(
                "Volume name cannot be empty".into(),
            ));
        }
        if s.contains('/') {
            return Err(LocketError::Validation(format!(
                "Volume name cannot contain slashes: '{}'",
                s
            )));
        }
        if s.contains('\0') {
            return Err(LocketError::Validation(
                "Volume name cannot contain null bytes".into(),
            ));
        }
        Ok(())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for VolumeName {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl Deref for VolumeName {
    type Target = str;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl fmt::Display for VolumeName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl TryFrom<String> for VolumeName {
    type Error = LocketError;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        Self::new(s)
    }
}

impl FromStr for VolumeName {
    type Err = LocketError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(s)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String")]
pub struct MountId(String);

impl MountId {
    pub fn new<S: Into<String>>(id: S) -> Result<Self, LocketError> {
        let s = id.into();
        if s.is_empty() {
            return Err(LocketError::Validation("Mount ID cannot be empty".into()));
        }
        Ok(Self(s))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for MountId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl Deref for MountId {
    type Target = str;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl fmt::Display for MountId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl TryFrom<String> for MountId {
    type Error = LocketError;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        Self::new(s)
    }
}

impl FromStr for MountId {
    type Err = LocketError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(s)
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
