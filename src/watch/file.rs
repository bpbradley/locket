use crate::secrets::SecretFileManager;
use std::path::PathBuf;
use async_trait::async_trait;
use super::{FsEvent, WatchHandler};

pub struct SecretFileWatcher<'a> {
    secrets: &'a mut SecretFileManager,
}

impl SecretFileWatcher<'_> {
    pub fn new<'a>(secrets: &'a mut SecretFileManager) -> SecretFileWatcher<'a> {
        SecretFileWatcher { secrets }
    }
}

#[async_trait]
impl<'a> WatchHandler for SecretFileWatcher<'a> {
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
