use crate::events::wait_for_signal;
use crate::{error::LocketError, path::AbsolutePath};
use hyper::server::conn::http1;
use hyper_util::rt::TokioIo;
use std::sync::Arc;
use tokio::net::UnixListener;
use tracing::{error, info};
pub mod api;
pub mod config;
pub mod driver;
pub mod error;
pub mod registry;
pub mod service;
pub mod types;

use crate::cmd::PluginConfig;
use registry::VolumeRegistry;
use service::DockerPluginService;

pub struct VolumePlugin {
    config: PluginConfig,
}

impl VolumePlugin {
    pub fn new(config: PluginConfig) -> Self {
        Self { config }
    }

    pub async fn run(self) -> Result<(), LocketError> {
        let socket_path = &self.config.socket;

        self.ensure_socket_path(socket_path).await?;
        let listener = UnixListener::bind(socket_path).map_err(LocketError::Io)?;

        let provider = self.config.provider.clone().build().await?;

        let driver = Arc::new(
            VolumeRegistry::new(
                self.config.state_dir.clone(),
                self.config.runtime_dir.clone(),
                provider,
            )
            .await?,
        );

        let service = DockerPluginService::new(driver);

        info!(socket=?socket_path, "Docker Plugin listening");

        let exit = wait_for_signal(false);
        tokio::pin!(exit);

        loop {
            tokio::select! {
                accept_result = listener.accept() => {
                    match accept_result {
                        Ok((stream, _addr)) => {
                            let io = TokioIo::new(stream);
                            let svc = service.clone();

                            tokio::task::spawn(async move {
                                if let Err(err) = http1::Builder::new().serve_connection(io, svc).await {
                                    error!("Error serving connection: {:?}", err);
                                }
                            });
                        }
                        Err(e) => error!("Socket accept error: {}", e),
                    }
                }

                _ = &mut exit => {
                    break;
                }
            }
        }

        Ok(())
    }

    async fn ensure_socket_path(&self, path: &AbsolutePath) -> Result<(), LocketError> {
        if path.exists() {
            info!("Removing existing socket file: {:?}", path);
            tokio::fs::remove_file(path)
                .await
                .map_err(LocketError::Io)?;
        }

        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(LocketError::Io)?;
        }
        Ok(())
    }
}

impl Drop for VolumePlugin {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.config.socket);
    }
}
