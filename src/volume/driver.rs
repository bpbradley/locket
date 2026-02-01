use super::error::PluginError;
use super::types::{DockerOptions, MountId, VolumeName};
use async_trait::async_trait;
use std::path::PathBuf;

#[async_trait]
pub trait VolumeDriver: Send + Sync {
    async fn create(&self, name: VolumeName, opts: DockerOptions) -> Result<(), PluginError>;
    async fn remove(&self, name: &VolumeName) -> Result<(), PluginError>;
    async fn mount(&self, name: &VolumeName, id: &MountId) -> Result<PathBuf, PluginError>;
    async fn unmount(&self, name: &VolumeName, id: &MountId) -> Result<(), PluginError>;
    async fn path(&self, name: &VolumeName) -> Result<PathBuf, PluginError>;
    async fn list(&self) -> Result<Vec<super::api::VolumeInfo>, PluginError>;
    async fn get(&self, name: &VolumeName) -> Result<Option<super::api::VolumeInfo>, PluginError>;
}
