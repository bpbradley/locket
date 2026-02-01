use crate::volume::types::DockerOptions;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct CreateRequest {
    pub name: String,
    pub opts: DockerOptions,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct NameRequest {
    pub name: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct MountRequest {
    pub name: String,
    #[serde(rename = "ID")]
    pub id: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct VolumeInfo {
    pub name: String,
    pub mountpoint: String,
    pub created_at: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct ListResponse {
    pub volumes: Vec<VolumeInfo>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct CapabilitiesResponse {
    pub capabilities: Capabilities,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct Capabilities {
    pub scope: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct MountResponse {
    pub mountpoint: String,
}

#[derive(Serialize)]
pub struct SuccessResponse {}
