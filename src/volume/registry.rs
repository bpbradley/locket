use super::{
    api::VolumeInfo,
    config::{VolumeArgs, VolumeSpec},
    driver::VolumeDriver,
    error::PluginError,
    types::{MountId, VolumeMount, VolumeName},
};
use crate::{
    config::Overlay,
    error::LocketError,
    events::StoppableHandler,
    path::{AbsolutePath, CanonicalPath},
    provider::{Provider, ProviderArgs, SecretsProvider},
    volume::types::DockerOptions,
    watch::FsWatcher,
};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::{RwLock, watch};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VolumeConfig {
    pub name: VolumeName,
    pub created_at: DateTime<Utc>,
    pub options: DockerOptions,
    // Skip serializing args; reconstruct from options on load
    #[serde(skip)]
    pub args: VolumeArgs,
}

pub struct WatcherHandle {
    task: JoinHandle<()>,
    token: CancellationToken,
}

pub struct ActiveResources {
    mounts: HashSet<MountId>,
    watcher: Option<WatcherHandle>,
}

pub enum VolumeLifecycle {
    Idle,
    Provisioning(watch::Sender<()>),
    Ready(ActiveResources),
}

pub struct Volume {
    pub config: VolumeConfig,
    pub mount: VolumeMount,
    pub state: VolumeLifecycle,
}

impl Volume {
    pub fn new(config: VolumeConfig, mount: VolumeMount) -> Self {
        Self {
            config,
            mount,
            state: VolumeLifecycle::Idle,
        }
    }

    fn mountpoint(&self) -> std::path::PathBuf {
        self.mount.path().to_path_buf()
    }

    fn to_info(&self) -> VolumeInfo {
        VolumeInfo {
            name: self.config.name.to_string(),
            mountpoint: self.mountpoint().display().to_string(),
            created_at: self.config.created_at.to_rfc3339(),
        }
    }
}

pub struct VolumeRegistry {
    state_file: AbsolutePath,
    runtime_dir: CanonicalPath,
    default_config: VolumeArgs,
    provider_cache: RwLock<HashMap<ProviderArgs, Arc<dyn SecretsProvider>>>,
    volumes: RwLock<HashMap<VolumeName, Arc<RwLock<Volume>>>>,
}

impl VolumeRegistry {
    pub async fn new(
        state_dir: AbsolutePath,
        runtime_dir: AbsolutePath,
        default_config: VolumeArgs,
    ) -> Result<Self, LocketError> {
        if let Err(e) = tokio::fs::create_dir_all(&runtime_dir).await {
            warn!("Failed to create runtime dir {:?}: {}", runtime_dir, e);
        }

        let registry = Self {
            state_file: state_dir.join("state.json"),
            runtime_dir: runtime_dir.canonicalize()?,
            default_config,
            provider_cache: RwLock::new(HashMap::new()),
            volumes: RwLock::new(HashMap::new()),
        };
        registry.load().await;
        Ok(registry)
    }

    async fn load(&self) {
        if !self.state_file.exists() {
            return;
        }

        if let Ok(data) = tokio::fs::read_to_string(&self.state_file).await
            && let Ok(configs) = serde_json::from_str::<Vec<VolumeConfig>>(&data)
        {
            let mut lock = self.volumes.write().await;
            for mut config in configs {
                let effective_args = self.default_config.clone().overlay(config.args.clone());
                config.args = VolumeArgs::try_from(config.options.clone()).unwrap_or_else(|e| {
                    tracing::warn!(volume=%config.name, "Failed to parse volume config: {}", e);
                    VolumeArgs::default()
                });
                let spec: VolumeSpec = match effective_args.try_into() {
                    Ok(s) => s,
                    Err(e) => {
                        warn!(volume=%config.name, "Failed to reconstruct spec from args: {}. Falling back to defaults.", e);
                        self.default_config.clone().try_into().unwrap_or_else(|e| {
                            warn!("Failed to parse configuration {}", e);
                            VolumeSpec::default()
                        })
                    }
                };

                let mountpoint = self.runtime_dir.join(config.name.as_str());

                let mount = VolumeMount::new(
                    mountpoint,
                    spec.mount.clone(),
                    spec.writer.get_user().cloned(),
                );

                let mut volume = Volume::new(config, mount);

                if volume.mount.is_mounted().await {
                    info!(volume=%volume.config.name, "Recovered existing mount");
                    if let Ok(resources) = self.provision_resources(&volume).await {
                        volume.state = VolumeLifecycle::Ready(resources);
                    }
                }

                lock.insert(volume.config.name.clone(), Arc::new(RwLock::new(volume)));
            }
        }
    }

    async fn persist(&self) -> Result<(), PluginError> {
        let lock = self.volumes.read().await;
        let mut list = Vec::new();
        for v in lock.values() {
            let volume = v.read().await;
            list.push(volume.config.clone());
        }

        let json = serde_json::to_string_pretty(&list).map_err(PluginError::Json)?;

        let tmp = self.state_file.with_extension("tmp");
        tokio::fs::write(&tmp, json)
            .await
            .map_err(|e| PluginError::Locket(LocketError::Io(e)))?;
        tokio::fs::rename(&tmp, &self.state_file)
            .await
            .map_err(|e| PluginError::Locket(LocketError::Io(e)))?;
        Ok(())
    }

    async fn provision_resources(&self, vol: &Volume) -> Result<ActiveResources, PluginError> {
        info!(volume=%vol.config.name, "Provisioning volume resources");

        let newly_mounted = if !vol.mount.is_mounted().await {
            vol.mount.mount().await?;
            true
        } else {
            false
        };

        let effective_args = self.default_config.clone().overlay(vol.config.args.clone());

        let effective_config: VolumeSpec = effective_args
            .clone()
            .try_into()
            .map_err(PluginError::Locket)?;

        let provider_args = effective_config.provider.clone();

        let provider = {
            let cache = self.provider_cache.read().await;
            if let Some(p) = cache.get(&effective_config.provider) {
                p.clone()
            } else {
                drop(cache);

                let provider_config: Provider = provider_args
                    .clone()
                    .try_into()
                    .map_err(PluginError::Locket)?;

                let new_p = provider_config
                    .build()
                    .await
                    .map_err(|e| PluginError::Locket(LocketError::Provider(e)))?;

                let mut cache = self.provider_cache.write().await;

                // re-check under write lock in case another thread built it first.
                if let Some(existing_p) = cache.get(&effective_config.provider) {
                    existing_p.clone()
                } else {
                    cache.insert(provider_args, new_p.clone());
                    new_p
                }
            }
        };

        let manager = effective_config
            .clone()
            .into_manager(AbsolutePath::from(vol.mount.path()), provider)?;

        if let Err(e) = manager.inject_all().await {
            error!("Injection failed: {}", e);
            if newly_mounted {
                let _ = vol.mount.unmount().await;
            }
            return Err(PluginError::Locket(LocketError::Secret(e)));
        }

        let watcher = if effective_config.watch {
            let token = CancellationToken::new();
            let handler = StoppableHandler::new(manager, token.clone());
            let watcher_svc = FsWatcher::new(std::time::Duration::from_millis(500), handler);

            let task = tokio::spawn(async move {
                if let Err(e) = watcher_svc.run().await {
                    error!("Watcher failed: {}", e);
                }
            });
            Some(WatcherHandle { task, token })
        } else {
            None
        };

        Ok(ActiveResources {
            mounts: HashSet::new(),
            watcher,
        })
    }
}

#[async_trait]
impl VolumeDriver for VolumeRegistry {
    async fn create(&self, name: VolumeName, opts: DockerOptions) -> Result<(), PluginError> {
        let args = VolumeArgs::try_from(opts.clone())?;

        let effective_args = self.default_config.clone().overlay(args.clone());

        let spec: VolumeSpec = effective_args
            .try_into()
            .map_err(|e: LocketError| PluginError::Validation(e.to_string()))?;

        let mut lock = self.volumes.write().await;
        if lock.contains_key(&name) {
            return Ok(());
        }

        let config = VolumeConfig {
            name: name.clone(),
            created_at: Utc::now(),
            args,
            options: opts,
        };

        let mountpoint = self.runtime_dir.join(name.as_str());

        let mount = VolumeMount::new(
            mountpoint.clone(),
            spec.mount.clone(),
            spec.writer.get_user().cloned(),
        );

        let volume = Volume::new(config, mount);
        lock.insert(name, Arc::new(RwLock::new(volume)));
        drop(lock);

        self.persist().await?;
        Ok(())
    }

    async fn remove(&self, name: &VolumeName) -> Result<(), PluginError> {
        let vol_arc = {
            let lock = self.volumes.read().await;
            lock.get(name).cloned().ok_or(PluginError::NotFound)?
        };

        {
            let vol = vol_arc.read().await;
            match &vol.state {
                VolumeLifecycle::Ready(active) if !active.mounts.is_empty() => {
                    return Err(PluginError::InUse);
                }
                VolumeLifecycle::Provisioning(_) => {
                    return Err(PluginError::InUse);
                }
                _ => {}
            }

            if vol.mount.is_mounted().await {
                return Err(PluginError::InUse);
            }
        }

        let mut lock = self.volumes.write().await;
        lock.remove(name);
        drop(lock);

        self.persist().await?;
        Ok(())
    }

    async fn mount(
        &self,
        name: &VolumeName,
        id: &MountId,
    ) -> Result<std::path::PathBuf, PluginError> {
        let vol_arc = {
            let lock = self.volumes.read().await;
            lock.get(name).cloned().ok_or(PluginError::NotFound)?
        };

        loop {
            let mut vol = vol_arc.write().await;

            match &mut vol.state {
                VolumeLifecycle::Ready(active) => {
                    active.mounts.insert(id.clone());
                    return Ok(vol.mountpoint());
                }

                VolumeLifecycle::Provisioning(tx) => {
                    let mut rx = tx.subscribe();
                    drop(vol); // Release lock
                    let _ = rx.changed().await;
                    continue; // Retry loop
                }

                VolumeLifecycle::Idle => {
                    let (tx, _) = watch::channel(());
                    vol.state = VolumeLifecycle::Provisioning(tx);

                    match self.provision_resources(&vol).await {
                        Ok(mut resources) => {
                            resources.mounts.insert(id.clone());
                            vol.state = VolumeLifecycle::Ready(resources);
                            return Ok(vol.mountpoint());
                        }
                        Err(e) => {
                            // Revert to Idle on failure
                            // The Provisioning channel implicitly closes here, notifying waiters
                            vol.state = VolumeLifecycle::Idle;
                            return Err(e);
                        }
                    }
                }
            }
        }
    }

    async fn unmount(&self, name: &VolumeName, id: &MountId) -> Result<(), PluginError> {
        let vol_arc = {
            let lock = self.volumes.read().await;
            lock.get(name).cloned().ok_or(PluginError::NotFound)?
        };

        let mut vol = vol_arc.write().await;

        let should_teardown = if let VolumeLifecycle::Ready(ref mut active) = vol.state {
            active.mounts.remove(id);
            active.mounts.is_empty()
        } else {
            false // Not ready or already unmounted?
        };

        if should_teardown {
            // Extract the active resources to destroy them.
            // replace state with Provisioning to block others while we tear down
            let (tx, _) = watch::channel(());
            let old_state = std::mem::replace(&mut vol.state, VolumeLifecycle::Provisioning(tx));

            if let VolumeLifecycle::Ready(active) = old_state {
                info!(volume=%vol.config.name, "Tearing down volume");

                if let Some(w) = active.watcher {
                    w.token.cancel();
                    let _ = w.task.await;
                }

                if let Err(e) = vol.mount.unmount().await {
                    error!("Unmount failed: {}", e);
                }
                vol.state = VolumeLifecycle::Idle;
            }
        }

        Ok(())
    }

    async fn path(&self, name: &VolumeName) -> Result<std::path::PathBuf, PluginError> {
        let lock = self.volumes.read().await;
        let vol_arc = lock.get(name).ok_or(PluginError::NotFound)?;
        let vol = vol_arc.read().await;
        Ok(vol.mountpoint())
    }

    async fn get(&self, name: &VolumeName) -> Result<Option<VolumeInfo>, PluginError> {
        let lock = self.volumes.read().await;
        if let Some(vol_arc) = lock.get(name) {
            let vol = vol_arc.read().await;
            Ok(Some(vol.to_info()))
        } else {
            Ok(None)
        }
    }

    async fn list(&self) -> Result<Vec<VolumeInfo>, PluginError> {
        let lock = self.volumes.read().await;
        let mut infos = Vec::new();
        for vol_arc in lock.values() {
            let vol = vol_arc.read().await;
            infos.push(vol.to_info());
        }
        Ok(infos)
    }
}
