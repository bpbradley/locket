//! File system watch handler for secret file events.
//! This module implements a `WatchHandler` that responds to file system
//! events by delegating to a `SecretFileManager` to handle secret updates.
//! It listens for write, remove, and move events on secret source files
//! and reflects those changes in the managed secret files.

use super::{FsEvent, WatchHandler};
use crate::secrets::SecretFileManager;
use async_trait::async_trait;
use std::path::PathBuf;

/// File system watch handler that delegates to SecretFileManager.
///
/// It responds to file system events by updating or removing
/// the corresponding secret files as needed. Its purpose is to
/// reflect changes in template source files to the managed secret files.
pub struct FileHandler {
    secrets: SecretFileManager,
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

    async fn handle(&mut self, events: Vec<FsEvent>) -> anyhow::Result<()> {
        for event in events {
            match event {
                FsEvent::Write(src) => self.secrets.handle_write(&src).await?,
                FsEvent::Remove(src) => self.secrets.handle_remove(&src)?,
                FsEvent::Move { from, to } => self.secrets.handle_move(&from, &to).await?,
            }
        }
        Ok(())
    }
}
