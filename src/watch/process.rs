use crate::env::EnvManager;
use std::path::PathBuf;
use secrecy::ExposeSecret;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use async_trait::async_trait;
use super::{FsEvent, WatchHandler};
use tracing::debug;
pub struct ProcessHandler {
    env: EnvManager,
    cmd: Vec<String>,
    env_hash: u64,
}

impl ProcessHandler {
    pub fn new(env: EnvManager, cmd: Vec<String>) -> Self {
        ProcessHandler { env, cmd, env_hash: 0 }
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

    async fn handle(&mut self, event: FsEvent) -> anyhow::Result<()> {
        match event {
            FsEvent::Remove(src) => {
                self.env.remove(&src);
            }
            FsEvent::Write(src) => {
                self.env.reload(&src).await?;
                let resolved = self.env.resolve().await?;
                let new_hash = Self::hash_env(&resolved);
                
                if new_hash != self.env_hash {
                    self.env_hash = new_hash;
                } else {
                    debug!("File changed but resolved environment is identical; skipping restart");
                }
            }
            _ => {}
        }
        Ok(())
    }
}
