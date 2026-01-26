use crate::volume::api::*;
use crate::volume::driver::VolumeDriver;
use crate::volume::error::PluginError;
use crate::volume::types::{MountId, VolumeName};

use http_body_util::{BodyExt, Full};
use hyper::body::{Bytes, Incoming};
use hyper::service::Service;
use hyper::{Request, Response, StatusCode};
use serde::Serialize;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tracing::info;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PluginRoute {
    Activate,
    Capabilities,
    Create,
    Get,
    List,
    Mount,
    Path,
    Remove,
    Unmount,
}

impl PluginRoute {
    fn from_path(path: &str) -> Option<Self> {
        match path {
            "/Plugin.Activate" => Some(Self::Activate),
            "/VolumeDriver.Capabilities" => Some(Self::Capabilities),
            "/VolumeDriver.Create" => Some(Self::Create),
            "/VolumeDriver.Get" => Some(Self::Get),
            "/VolumeDriver.List" => Some(Self::List),
            "/VolumeDriver.Mount" => Some(Self::Mount),
            "/VolumeDriver.Path" => Some(Self::Path),
            "/VolumeDriver.Remove" => Some(Self::Remove),
            "/VolumeDriver.Unmount" => Some(Self::Unmount),
            _ => None,
        }
    }
}

#[derive(Clone)]
pub struct DockerPluginService {
    driver: Arc<dyn VolumeDriver>,
}

impl DockerPluginService {
    pub fn new(driver: Arc<dyn VolumeDriver>) -> Self {
        Self { driver }
    }

    async fn handle(&self, req: Request<Incoming>) -> Result<Response<Full<Bytes>>, PluginError> {
        let path = req.uri().path();
        info!(method = ?req.method(), path = path, "Received request");

        let route = match PluginRoute::from_path(path) {
            Some(r) => r,
            None => {
                let mut not_found = Response::new(Full::default());
                *not_found.status_mut() = StatusCode::NOT_FOUND;
                return Ok(not_found);
            }
        };

        match route {
            PluginRoute::Activate => self.handle_activate().await,
            PluginRoute::Capabilities => self.handle_capabilities().await,
            PluginRoute::Create => self.handle_create(req).await,
            PluginRoute::Get => self.handle_get(req).await,
            PluginRoute::List => self.handle_list().await,
            PluginRoute::Mount => self.handle_mount(req).await,
            PluginRoute::Path => self.handle_path(req).await,
            PluginRoute::Remove => self.handle_remove(req).await,
            PluginRoute::Unmount => self.handle_unmount(req).await,
        }
    }

    async fn handle_activate(&self) -> Result<Response<Full<Bytes>>, PluginError> {
        let resp = PluginActivateResponse {
            implements: vec!["VolumeDriver".to_string()],
        };
        Ok(json_ok(&resp))
    }

    async fn handle_capabilities(&self) -> Result<Response<Full<Bytes>>, PluginError> {
        let resp = CapabilitiesResponse {
            capabilities: Capabilities {
                scope: "local".into(),
            },
        };
        Ok(json_ok(&resp))
    }

    async fn handle_create(
        &self,
        req: Request<Incoming>,
    ) -> Result<Response<Full<Bytes>>, PluginError> {
        let req: CreateRequest = decode(req).await?;
        let name = VolumeName::new(req.name)?;

        info!("Creating volume: {}", name);
        self.driver.create(name, req.opts).await?;
        Ok(json_ok(&SuccessResponse {}))
    }

    async fn handle_remove(
        &self,
        req: Request<Incoming>,
    ) -> Result<Response<Full<Bytes>>, PluginError> {
        let req: NameRequest = decode(req).await?;
        let name = VolumeName::new(req.name)?;

        info!("Removing volume: {}", name);
        self.driver.remove(&name).await?;
        Ok(json_ok(&SuccessResponse {}))
    }

    async fn handle_mount(
        &self,
        req: Request<Incoming>,
    ) -> Result<Response<Full<Bytes>>, PluginError> {
        let req: MountRequest = decode(req).await?;
        let name = VolumeName::new(req.name)?;
        let id = MountId::new(req.id)?;

        info!("Mounting volume: {} (id: {})", name, id);
        let path = self.driver.mount(&name, &id).await?;

        Ok(json_ok(&MountResponse {
            mountpoint: path.to_string_lossy().to_string(),
        }))
    }

    async fn handle_unmount(
        &self,
        req: Request<Incoming>,
    ) -> Result<Response<Full<Bytes>>, PluginError> {
        let req: MountRequest = decode(req).await?;
        let name = VolumeName::new(req.name)?;
        let id = MountId::new(req.id)?;

        info!("Unmounting volume: {} (id: {})", name, id);
        self.driver.unmount(&name, &id).await?;
        Ok(json_ok(&SuccessResponse {}))
    }

    async fn handle_path(
        &self,
        req: Request<Incoming>,
    ) -> Result<Response<Full<Bytes>>, PluginError> {
        let req: NameRequest = decode(req).await?;
        let name = VolumeName::new(req.name)?;

        let mp = self.driver.path(&name).await?;
        Ok(json_ok(&MountResponse {
            mountpoint: mp.to_string_lossy().to_string(),
        }))
    }

    async fn handle_list(&self) -> Result<Response<Full<Bytes>>, PluginError> {
        let volumes = self.driver.list().await?;
        Ok(json_ok(&ListResponse { volumes }))
    }

    async fn handle_get(
        &self,
        req: Request<Incoming>,
    ) -> Result<Response<Full<Bytes>>, PluginError> {
        let req: NameRequest = decode(req).await?;
        let name = VolumeName::new(req.name)?;

        let info = self.driver.get(&name).await?.ok_or(PluginError::NotFound)?;
        Ok(json_ok(&HashMap::from([("Volume", info)])))
    }
}

impl Service<Request<Incoming>> for DockerPluginService {
    type Response = Response<Full<Bytes>>;
    type Error = hyper::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn call(&self, req: Request<Incoming>) -> Self::Future {
        let svc = self.clone();
        Box::pin(async move {
            match svc.handle(req).await {
                Ok(resp) => Ok(resp),
                Err(e) => Ok(e.into_response()),
            }
        })
    }
}

async fn decode<T: serde::de::DeserializeOwned>(req: Request<Incoming>) -> Result<T, PluginError> {
    let body_bytes = req
        .collect()
        .await
        .map_err(|e| PluginError::Internal(e.to_string()))?
        .to_bytes();
    serde_json::from_slice(&body_bytes).map_err(PluginError::Json)
}

fn json_ok<T: Serialize>(data: &T) -> Response<Full<Bytes>> {
    let json = serde_json::to_vec(data).expect("Serialization failed");
    Response::builder()
        .header("Content-Type", "application/json")
        .body(Full::new(Bytes::from(json)))
        .unwrap()
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct PluginActivateResponse {
    implements: Vec<String>,
}
