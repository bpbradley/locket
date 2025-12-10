use crate::secrets::SecretFileManager;
use std::path::PathBuf;
use async_trait::async_trait;
use super::{FsEvent, WatchHandler};

pub struct FileHandler {
    secrets: SecretFileManager
}

impl FileHandler {
    pub fn new(secrets: SecretFileManager) -> Self {
        FileHandler { secrets }
    }
}

#[async_trait]
impl WatchHandler for FileHandler {
    fn paths(&self) -> Vec<PathBuf> {
        self.secrets.sources()
    }

    async fn handle(&mut self, event: FsEvent) -> anyhow::Result<()> {
        match event {
            FsEvent::Write(src) => self.secrets.handle_write(&src).await?,
            FsEvent::Remove(src) => self.secrets.handle_remove(&src)?,
            FsEvent::Move { from, to } => self.secrets.handle_move(&from, &to).await?,
        }
        Ok(())
    }
}
