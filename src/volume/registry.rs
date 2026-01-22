use super::{
    api::VolumeInfo,
    config::{VolumeArgs, VolumeSpec},
    driver::VolumeDriver,
    error::PluginError,
    types::{MountId, VolumeMount, VolumeName},
};
use crate::{
    error::LocketError,
    events::StoppableHandler,
    path::{AbsolutePath, CanonicalPath},
    provider::SecretsProvider,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum VolumeLifecycle {
    #[default]
    Idle,
    Provisioning,
    Ready,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct VolumeMetadata {
    name: VolumeName,
    options: HashMap<String, String>,
    created_at: DateTime<Utc>,
}

#[derive(Debug)]
struct ActiveVolume {
    mount_ids: HashSet<MountId>,
    state_tx: watch::Sender<VolumeLifecycle>,
    watcher_task: Option<JoinHandle<()>>,
    shutdown_token: Option<CancellationToken>,
}

impl Default for ActiveVolume {
    fn default() -> Self {
        let (tx, _rx) = watch::channel(VolumeLifecycle::Idle);
        Self {
            mount_ids: HashSet::new(),
            state_tx: tx,
            watcher_task: None,
            shutdown_token: None,
        }
    }
}

// Helper to get current state from the channel
impl ActiveVolume {
    fn lifecycle(&self) -> VolumeLifecycle {
        *self.state_tx.borrow()
    }

    fn set_lifecycle(&self, state: VolumeLifecycle) {
        let _ = self.state_tx.send(state);
    }
}

#[derive(Debug)]
struct VolumeEntry {
    meta: VolumeMetadata,
    spec: VolumeSpec,
    mount: VolumeMount,
    state: ActiveVolume,
}

impl VolumeEntry {
    fn mountpoint(&self) -> std::path::PathBuf {
        self.mount.path().to_path_buf()
    }

    fn to_info(&self) -> VolumeInfo {
        let mut status = HashMap::new();
        status.insert("Mounts".to_string(), self.state.mount_ids.len().to_string());
        status.insert("State".to_string(), format!("{:?}", self.state.lifecycle()));
        for (k, v) in &self.meta.options {
            status.insert(format!("Option.{}", k), v.clone());
        }
        VolumeInfo {
            name: self.meta.name.to_string(),
            mountpoint: self.mountpoint().display().to_string(),
            created_at: self.meta.created_at.to_rfc3339(),
            status,
        }
    }
}

pub struct VolumeRegistry {
    state_file: AbsolutePath,
    runtime_dir: CanonicalPath,
    provider: Arc<dyn SecretsProvider>,
    entries: RwLock<HashMap<VolumeName, Arc<RwLock<VolumeEntry>>>>,
}

impl VolumeRegistry {
    pub async fn new(
        state_dir: AbsolutePath,
        runtime_dir: AbsolutePath,
        provider: Arc<dyn SecretsProvider>,
    ) -> Result<Self, LocketError> {
        let state_file = state_dir.join("state.json");

        if let Err(e) = tokio::fs::create_dir_all(&runtime_dir).await {
            warn!("Failed to create runtime dir {:?}: {}", runtime_dir, e);
        }

        let runtime_dir = runtime_dir.canonicalize()?;

        let registry = Self {
            state_file,
            runtime_dir,
            provider,
            entries: RwLock::new(HashMap::new()),
        };

        registry.load().await;
        Ok(registry)
    }

    async fn load(&self) {
        if !self.state_file.exists() {
            return;
        }

        match tokio::fs::read_to_string(&self.state_file).await {
            Ok(data) => match serde_json::from_str::<Vec<VolumeMetadata>>(&data) {
                Ok(list) => {
                    let mut lock = self.entries.write().await;
                    let mut loaded = 0;
                    for meta in list {
                        if let Ok(entry) = self.rehydrate_entry(meta).await {
                            let mut entry = entry;

                            if entry.mount.is_mounted().await {
                                info!(volume = %entry.meta.name, "Recovered existing mount during startup");
                                if let Err(e) = self.provision_internal(&mut entry).await {
                                    error!(volume = %entry.meta.name, "Failed to re-provision recovered volume: {}", e);
                                } else {
                                    entry.state.set_lifecycle(VolumeLifecycle::Ready);
                                }
                            }

                            lock.insert(entry.meta.name.clone(), Arc::new(RwLock::new(entry)));
                            loaded += 1;
                        }
                    }
                    info!("Loaded {} volumes from state", loaded);
                }
                Err(e) => warn!("State file corruption: {}", e),
            },
            Err(e) => warn!("Failed to read state file: {}", e),
        }
    }

    async fn rehydrate_entry(&self, meta: VolumeMetadata) -> Result<VolumeEntry, ()> {
        let args = VolumeArgs::try_from(meta.options.clone()).map_err(|e| {
            error!("Failed to parse options for {}: {}", meta.name, e);
        })?;
        let spec: VolumeSpec = args.try_into().map_err(|e| {
            error!("Invalid config for {}: {}", meta.name, e);
        })?;

        let mountpoint = self.runtime_dir.join(meta.name.as_str());
        let mount = VolumeMount::new(
            mountpoint.clone(),
            spec.mount.clone(),
            spec.writer.get_user().cloned(),
        );

        Ok(VolumeEntry {
            meta,
            spec,
            mount,
            state: ActiveVolume::default(),
        })
    }

    async fn persist(&self) -> Result<(), PluginError> {
        let lock = self.entries.read().await;
        let mut list = Vec::new();
        for v in lock.values() {
            let entry = v.read().await;
            list.push(entry.meta.clone());
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

    async fn provision_internal(&self, entry: &mut VolumeEntry) -> Result<(), PluginError> {
        let name = &entry.meta.name;
        info!(volume=%name, "Provisioning volume resources");

        let newly_mounted = if !entry.mount.is_mounted().await {
            entry.mount.mount().await?;
            true
        } else {
            info!(volume=%name, "Volume already mounted, skipping mount");
            false
        };

        let manager = entry.spec.clone().into_manager(
            AbsolutePath::from(entry.mount.path()),
            self.provider.clone(),
        )?;

        if let Err(e) = manager.inject_all().await {
            error!("Injection failed: {}", e);
            if newly_mounted {
                let _ = entry.mount.unmount().await;
            }
            return Err(PluginError::Locket(LocketError::Secret(e)));
        }

        if entry.spec.watch {
            let token = CancellationToken::new();
            let handler = StoppableHandler::new(manager, token.clone());
            let watcher = FsWatcher::new(std::time::Duration::from_millis(500), handler);

            let task = tokio::spawn(async move {
                if let Err(e) = watcher.run().await {
                    error!("Watcher failed: {}", e);
                }
            });

            entry.state.watcher_task = Some(task);
            entry.state.shutdown_token = Some(token);
        }

        Ok(())
    }

    async fn teardown_internal(&self, entry: &mut VolumeEntry) -> Result<(), PluginError> {
        info!(volume=%entry.meta.name, "Tearing down volume");

        if let Some(token) = entry.state.shutdown_token.take() {
            token.cancel();
        }

        if let Some(task) = entry.state.watcher_task.take() {
            let _ = task.await;
        }

        entry.mount.unmount().await?;
        Ok(())
    }
}

#[async_trait]
impl VolumeDriver for VolumeRegistry {
    async fn create(
        &self,
        name: VolumeName,
        opts: HashMap<String, String>,
    ) -> Result<(), PluginError> {
        let args = VolumeArgs::try_from(opts.clone())?;
        let spec: VolumeSpec = args.try_into()?;

        let mut lock = self.entries.write().await;
        if lock.contains_key(&name) {
            return Ok(());
        }

        let meta = VolumeMetadata {
            name: name.clone(),
            options: opts,
            created_at: Utc::now(),
        };

        let mountpoint = self.runtime_dir.join(name.as_str());
        let mount = VolumeMount::new(
            mountpoint.clone(),
            spec.mount.clone(),
            spec.writer.get_user().cloned(),
        );

        lock.insert(
            name,
            Arc::new(RwLock::new(VolumeEntry {
                meta,
                spec,
                mount,
                state: ActiveVolume::default(),
            })),
        );
        drop(lock);

        self.persist().await?;
        Ok(())
    }

    async fn remove(&self, name: &VolumeName) -> Result<(), PluginError> {
        let entry_arc = {
            let lock = self.entries.read().await;
            lock.get(name).cloned().ok_or(PluginError::NotFound)?
        };

        {
            let entry = entry_arc.read().await;
            if !entry.state.mount_ids.is_empty() {
                return Err(PluginError::InUse);
            }
            if entry.mount.is_mounted().await {
                return Err(PluginError::InUse);
            }
        }

        let mut lock = self.entries.write().await;
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
        let entry_arc = {
            let lock = self.entries.read().await;
            lock.get(name).cloned().ok_or(PluginError::NotFound)?
        };

        loop {
            let mut entry = entry_arc.write().await;

            match entry.state.lifecycle() {
                VolumeLifecycle::Ready => {
                    entry.state.mount_ids.insert(id.clone());
                    return Ok(entry.mountpoint());
                }
                VolumeLifecycle::Provisioning => {
                    let mut rx = entry.state.state_tx.subscribe();
                    drop(entry);

                    info!(volume=%name, "Waiting for existing provisioning to complete...");
                    let _ = rx.changed().await;
                    continue;
                }
                VolumeLifecycle::Idle => {
                    entry.state.set_lifecycle(VolumeLifecycle::Provisioning);
                    entry.state.mount_ids.insert(id.clone());

                    match self.provision_internal(&mut entry).await {
                        Ok(_) => {
                            entry.state.set_lifecycle(VolumeLifecycle::Ready);
                            return Ok(entry.mountpoint());
                        }
                        Err(e) => {
                            entry.state.set_lifecycle(VolumeLifecycle::Idle);
                            entry.state.mount_ids.remove(id);
                            return Err(e);
                        }
                    }
                }
            }
        }
    }

    async fn unmount(&self, name: &VolumeName, id: &MountId) -> Result<(), PluginError> {
        let entry_arc = {
            let lock = self.entries.read().await;
            lock.get(name).cloned().ok_or(PluginError::NotFound)?
        };

        let mut entry = entry_arc.write().await;

        entry.state.mount_ids.remove(id);

        if entry.state.mount_ids.is_empty() {
            entry.state.set_lifecycle(VolumeLifecycle::Provisioning);

            match self.teardown_internal(&mut entry).await {
                Ok(_) => {
                    entry.state.set_lifecycle(VolumeLifecycle::Idle);
                }
                Err(e) => {
                    error!("Teardown failed: {}", e);
                    entry.state.set_lifecycle(VolumeLifecycle::Idle);
                    return Err(e);
                }
            }
        }

        Ok(())
    }

    async fn path(&self, name: &VolumeName) -> Result<std::path::PathBuf, PluginError> {
        let lock = self.entries.read().await;
        let entry_arc = lock.get(name).ok_or(PluginError::NotFound)?;
        let entry = entry_arc.read().await;
        Ok(entry.mountpoint())
    }

    async fn get(&self, name: &VolumeName) -> Result<Option<VolumeInfo>, PluginError> {
        let lock = self.entries.read().await;
        if let Some(entry_arc) = lock.get(name) {
            let entry = entry_arc.read().await;
            Ok(Some(entry.to_info()))
        } else {
            Ok(None)
        }
    }

    async fn list(&self) -> Result<Vec<VolumeInfo>, PluginError> {
        let lock = self.entries.read().await;
        let mut infos = Vec::new();
        for entry_arc in lock.values() {
            let entry = entry_arc.read().await;
            infos.push(entry.to_info());
        }
        Ok(infos)
    }
}
