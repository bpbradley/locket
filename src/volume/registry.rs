use super::{
    api::VolumeInfo,
    config::{VolumeArgs, VolumeSpec},
    driver::VolumeDriver,
    error::PluginError,
    types::{MountId, VolumeName},
};
use crate::{
    error::LocketError,
    events::{EventHandler, FsEvent, HandlerError},
    path::{AbsolutePath, CanonicalPath},
    provider::SecretsProvider,
    secrets::SecretFileManager,
    watch::FsWatcher,
};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use futures::future::BoxFuture;
use nix::mount::{MntFlags, MsFlags, mount, umount2};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::{Notify, RwLock, watch};
use tokio::task::JoinHandle;
use tracing::{error, info, warn};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum VolumeLifecycle {
    /// Volume is created but not mounted on host
    #[default]
    Idle,
    /// Currently mounting/injecting. Access is blocked for other containers.
    Provisioning,
    /// Mounted, secrets injected, and ready for use.
    Ready,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct VolumeMetadata {
    name: VolumeName,
    options: HashMap<String, String>,
    created_at: DateTime<Utc>,
}

#[derive(Debug, Default)]
struct ActiveVolume {
    mount_ids: HashSet<MountId>,
    lifecycle: VolumeLifecycle,
    notify: Arc<Notify>,
    watcher_task: Option<JoinHandle<()>>,
    shutdown_tx: Option<watch::Sender<bool>>,
}

#[derive(Debug)]
struct VolumeEntry {
    meta: VolumeMetadata,
    spec: VolumeSpec,
    state: ActiveVolume,
}

impl VolumeEntry {
    fn mountpoint(&self, root_dir: &Path) -> PathBuf {
        root_dir.join(self.meta.name.as_str())
    }

    fn to_info(&self, root_dir: &Path) -> VolumeInfo {
        let mut status = HashMap::new();
        status.insert("Mounts".to_string(), self.state.mount_ids.len().to_string());
        for (k, v) in &self.meta.options {
            status.insert(format!("Option.{}", k), v.clone());
        }
        VolumeInfo {
            name: self.meta.name.to_string(),
            mountpoint: self.mountpoint(root_dir).to_string_lossy().to_string(),
            created_at: self.meta.created_at.to_rfc3339(),
            status,
        }
    }
}

pub struct VolumeRegistry {
    state_file: AbsolutePath,
    runtime_dir: CanonicalPath,
    provider: Arc<dyn SecretsProvider>,
    entries: RwLock<HashMap<VolumeName, VolumeEntry>>,
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
                        let args = match VolumeArgs::try_from(meta.options.clone()) {
                            Ok(a) => a,
                            Err(e) => {
                                error!("Failed to parse options for {}: {}", meta.name, e);
                                continue;
                            }
                        };
                        let spec = match args.try_into() {
                            Ok(s) => s,
                            Err(e) => {
                                error!("Invalid config for {}: {}", meta.name, e);
                                continue;
                            }
                        };

                        lock.insert(
                            meta.name.clone(),
                            VolumeEntry {
                                meta,
                                spec,
                                state: ActiveVolume::default(),
                            },
                        );
                        loaded += 1;
                    }
                    info!("Loaded {} volumes from state", loaded);
                }
                Err(e) => warn!("State file corruption: {}", e),
            },
            Err(e) => warn!("Failed to read state file: {}", e),
        }
    }

    async fn persist(&self) -> Result<(), PluginError> {
        let lock = self.entries.read().await;
        let list: Vec<&VolumeMetadata> = lock.values().map(|v| &v.meta).collect();
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

    async fn provision(&self, name: &VolumeName, mountpoint: &Path) -> Result<(), PluginError> {
        info!(volume=%name, "Provisioning volume resources");

        if !tokio::fs::try_exists(mountpoint).await.unwrap_or(false) {
            tokio::fs::create_dir_all(mountpoint)
                .await
                .map_err(LocketError::Io)?;
        }

        let spec = {
            let lock = self.entries.read().await;
            lock.get(name).ok_or(PluginError::NotFound)?.spec.clone()
        };

        let watch_enabled = spec.watch;
        let target_mount = mountpoint.to_path_buf();
        let data = format!("size={},mode={}", spec.mount.size, spec.mount.mode);
        let flags: MsFlags = spec.mount.flags.clone().into();
        tokio::task::spawn_blocking(move || {
            let fstype = Some("tmpfs");
            let src = Some("tmpfs");
            mount(src, &target_mount, fstype, flags, Some(data.as_str()))
        })
        .await
        .map_err(|e| PluginError::Internal(format!("Join error: {}", e)))?
        .map_err(|e| PluginError::Internal(format!("Mount failed: {}", e)))?;
        if let Some(user) = spec.writer.get_user() {
            let (u, g) = user.as_nix();
            let target_chown = mountpoint.to_path_buf();
            tokio::task::spawn_blocking(move || {
                nix::unistd::chown(&target_chown, Some(u), Some(g))
            })
            .await
            .map_err(|_| PluginError::Internal("Join error".into()))?
            .map_err(|e| PluginError::Internal(format!("Chown failed: {}", e)))?;
        }

        let manager = spec.into_manager(AbsolutePath::from(mountpoint), self.provider.clone())?;

        if let Err(e) = manager.inject_all().await {
            error!("Injection failed: {}", e);
            let target_cleanup = mountpoint.to_path_buf();
            let _ =
                tokio::task::spawn_blocking(move || umount2(&target_cleanup, MntFlags::MNT_DETACH))
                    .await;
            return Err(PluginError::Locket(LocketError::Secret(e)));
        }

        if watch_enabled {
            let (tx, rx) = watch::channel(false);
            let adapter = VolumeEventHandler {
                inner: manager,
                stop_rx: rx,
            };
            let watcher = FsWatcher::new(std::time::Duration::from_millis(500), adapter);
            let task = tokio::spawn(async move {
                if let Err(e) = watcher.run().await {
                    error!("Watcher failed: {}", e);
                }
            });

            let mut lock = self.entries.write().await;
            if let Some(entry) = lock.get_mut(name) {
                entry.state.watcher_task = Some(task);
                entry.state.shutdown_tx = Some(tx);
            }
        }

        Ok(())
    }

    async fn teardown(&self, name: &VolumeName, mountpoint: &Path) -> Result<(), PluginError> {
        info!(volume=%name, "Tearing down volume");

        let tx = {
            let mut lock = self.entries.write().await;
            if let Some(entry) = lock.get_mut(name) {
                entry.state.shutdown_tx.take()
            } else {
                None
            }
        };
        if let Some(tx) = tx {
            let _ = tx.send(true);
        }

        let target = mountpoint.to_path_buf();
        tokio::task::spawn_blocking(move || umount2(&target, MntFlags::MNT_DETACH))
            .await
            .map_err(|_| PluginError::Internal("Join error".into()))?
            .map_err(|e| PluginError::Internal(format!("Unmount failed: {}", e)))?;

        if tokio::fs::try_exists(mountpoint).await.unwrap_or(false)
            && let Err(e) = tokio::fs::remove_dir_all(mountpoint).await
        {
            warn!("Failed to remove volume dir: {}", e);
        }
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

        lock.insert(
            name,
            VolumeEntry {
                meta,
                spec,
                state: ActiveVolume::default(),
            },
        );
        drop(lock);

        self.persist().await?;
        Ok(())
    }

    async fn remove(&self, name: &VolumeName) -> Result<(), PluginError> {
        let mut lock = self.entries.write().await;
        if let Some(entry) = lock.get(name) {
            if !entry.state.mount_ids.is_empty() {
                return Err(PluginError::InUse);
            }
        } else {
            return Err(PluginError::NotFound);
        }
        lock.remove(name);
        drop(lock);
        self.persist().await?;
        Ok(())
    }

    async fn mount(&self, name: &VolumeName, id: &MountId) -> Result<PathBuf, PluginError> {
        loop {
            let (mountpoint, action, notify) = {
                let mut lock = self.entries.write().await;
                let entry = lock.get_mut(name).ok_or(PluginError::NotFound)?;
                let path = entry.mountpoint(&self.runtime_dir);

                match entry.state.lifecycle {
                    VolumeLifecycle::Ready => {
                        entry.state.mount_ids.insert(id.clone());
                        return Ok(path);
                    }
                    VolumeLifecycle::Provisioning => (
                        path,
                        VolumeLifecycle::Provisioning,
                        entry.state.notify.clone(),
                    ),
                    VolumeLifecycle::Idle => {
                        entry.state.lifecycle = VolumeLifecycle::Provisioning;
                        entry.state.mount_ids.insert(id.clone());
                        (path, VolumeLifecycle::Idle, entry.state.notify.clone())
                    }
                }
            };

            match action {
                VolumeLifecycle::Ready => unreachable!(),

                VolumeLifecycle::Provisioning => {
                    info!(volume=%name, "Waiting for existing provisioning to complete...");
                    notify.notified().await;
                    continue;
                }

                VolumeLifecycle::Idle => match self.provision(name, &mountpoint).await {
                    Ok(_) => {
                        let mut lock = self.entries.write().await;
                        if let Some(entry) = lock.get_mut(name) {
                            entry.state.lifecycle = VolumeLifecycle::Ready;
                        }
                        notify.notify_waiters();
                        return Ok(mountpoint);
                    }
                    Err(e) => {
                        let mut lock = self.entries.write().await;
                        if let Some(entry) = lock.get_mut(name) {
                            entry.state.lifecycle = VolumeLifecycle::Idle;
                            entry.state.mount_ids.remove(id);
                        }
                        notify.notify_waiters();
                        return Err(e);
                    }
                },
            }
        }
    }

    async fn unmount(&self, name: &VolumeName, id: &MountId) -> Result<(), PluginError> {
        let (mountpoint, needs_teardown) = {
            let mut lock = self.entries.write().await;
            let entry = lock.get_mut(name).ok_or(PluginError::NotFound)?;

            entry.state.mount_ids.remove(id);

            let empty = entry.state.mount_ids.is_empty();
            let path = entry.mountpoint(&self.runtime_dir);

            if empty {
                entry.state.lifecycle = VolumeLifecycle::Provisioning;
            }

            (path, empty)
        };

        if needs_teardown {
            match self.teardown(name, &mountpoint).await {
                Ok(_) => {
                    let mut lock = self.entries.write().await;
                    if let Some(entry) = lock.get_mut(name) {
                        entry.state.lifecycle = VolumeLifecycle::Idle;
                        entry.state.notify.notify_waiters();
                    }
                }
                Err(e) => {
                    error!("Teardown failed: {}", e);
                    let mut lock = self.entries.write().await;
                    if let Some(entry) = lock.get_mut(name) {
                        entry.state.lifecycle = VolumeLifecycle::Idle;
                        entry.state.notify.notify_waiters();
                    }
                    return Err(e);
                }
            }
        }

        Ok(())
    }

    async fn path(&self, name: &VolumeName) -> Result<PathBuf, PluginError> {
        let lock = self.entries.read().await;
        let entry = lock.get(name).ok_or(PluginError::NotFound)?;
        Ok(entry.mountpoint(&self.runtime_dir))
    }

    async fn get(&self, name: &VolumeName) -> Result<Option<VolumeInfo>, PluginError> {
        let lock = self.entries.read().await;
        Ok(lock.get(name).map(|entry| entry.to_info(&self.runtime_dir)))
    }

    async fn list(&self) -> Result<Vec<VolumeInfo>, PluginError> {
        let lock = self.entries.read().await;
        Ok(lock
            .values()
            .map(|entry| entry.to_info(&self.runtime_dir))
            .collect())
    }
}

struct VolumeEventHandler {
    inner: SecretFileManager,
    stop_rx: watch::Receiver<bool>,
}

#[async_trait]
impl EventHandler for VolumeEventHandler {
    fn paths(&self) -> Vec<AbsolutePath> {
        self.inner.paths()
    }

    async fn handle(&mut self, events: Vec<FsEvent>) -> Result<(), HandlerError> {
        self.inner.handle(events).await
    }

    fn wait(&self) -> BoxFuture<'static, Result<(), HandlerError>> {
        let mut rx = self.stop_rx.clone();
        Box::pin(async move {
            let _ = rx.changed().await;
            Ok(())
        })
    }
}
