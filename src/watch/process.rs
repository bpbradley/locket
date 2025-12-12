use super::{FsEvent, WatchHandler};
use crate::env::EnvManager;
use async_trait::async_trait;
use secrecy::ExposeSecret;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use tracing::debug;
pub struct ProcessHandler {
    env: EnvManager,
    cmd: Vec<String>,
    env_hash: u64,
}

impl ProcessHandler {
    pub fn new(env: EnvManager, cmd: Vec<String>) -> Self {
        ProcessHandler {
            env,
            cmd,
            env_hash: 0,
        }
    }
    fn hash_env(map: &std::collections::HashMap<String, secrecy::SecretString>) -> u64 {
        let mut hasher = DefaultHasher::new();
        let mut keys: Vec<_> = map.keys().collect();
        keys.sort();

        for k in keys {
            k.hash(&mut hasher);
            map.get(k).unwrap().expose_secret().hash(&mut hasher);
        }
        hasher.finish()
    }
}

#[async_trait]
impl WatchHandler for ProcessHandler {
    fn paths(&self) -> Vec<PathBuf> {
        self.env.files()
    }

    async fn handle(&mut self, events: Vec<FsEvent>) -> anyhow::Result<()> {
        if events.is_empty() {
            return Ok(());
        }
        match self.env.resolve().await {
            Ok(resolved) => {
                let new_hash = Self::hash_env(&resolved);
                if new_hash != self.env_hash {
                    self.env_hash = new_hash;
                    tracing::info!(
                        "Environment changed ({} events), restarting process...",
                        events.len()
                    );

                    // TODO: Trigger child process restart here?
                } else {
                    debug!("Files changed but resolved environment is identical; skipping restart");
                }
            }
            Err(e) => {
                // Log but don't crash the watcher loop
                tracing::error!("Failed to reload environment: {}", e);
            }
        }
        Ok(())
    }
}
