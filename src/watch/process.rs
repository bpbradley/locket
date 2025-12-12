use super::{FsEvent, WatchHandler};
use crate::env::EnvManager;
use async_trait::async_trait;
use secrecy::ExposeSecret;
use std::collections::{hash_map::DefaultHasher, HashMap};
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use tokio::process::{Child, Command};
use thiserror::Error;
use tracing::{debug, info, error};
use std::process::ExitStatus;

#[derive(Debug, Error)]
pub enum ExecError {
    #[error("child process failed with code {0}")]
    Code(i32),

    #[error("child process terminated by signal")]
    Signal,

    #[error("failed to wait on child: {0}")]
    Io(#[from] std::io::Error),

    #[error("no child process is currently running")]
    NoChild,
}

impl From<ExitStatus> for ExecError {
    fn from(status: ExitStatus) -> Self {
        if let Some(code) = status.code() {
            Self::Code(code)
        } else {
            Self::Signal
        }
    }
}

pub struct ProcessHandler {
    env: EnvManager,
    cmd: Vec<String>,
    env_hash: u64,
    child: Option<Child>,
}

impl ProcessHandler {
    pub fn new(env: EnvManager, cmd: Vec<String>) -> Self {
        ProcessHandler {
            env,
            cmd,
            env_hash: 0,
            child: None,
        }
    }

    fn hash_env(map: &HashMap<String, secrecy::SecretString>) -> u64 {
        let mut hasher = DefaultHasher::new();
        let mut keys: Vec<_> = map.keys().collect();
        keys.sort();

        for k in keys {
            k.hash(&mut hasher);
            map.get(k).unwrap().expose_secret().hash(&mut hasher);
        }
        hasher.finish()
    }

    async fn restart(&mut self, env_map: &HashMap<String, secrecy::SecretString>) -> anyhow::Result<()> {
        // Kill the old process if it exists
        if let Some(mut child) = self.child.take() {
            debug!("Killing old process (pid: {:?})", child.id());
            
            // Standard Tokio kill sends SIGKILL (immediate termination).
            // TODO: implement graceful shutdown with SIGTERM first?
            child.kill().await?;
            
            child.wait().await?;
        }

        if self.cmd.is_empty() {
            return Ok(());
        }

        // Prepare command
        // We use the first arg as the program, and the rest as arguments.
        let mut command = Command::new(&self.cmd[0]);
        command.args(&self.cmd[1..]);

        // Inject secrets
        // We must expose the secrets here to pass them to the OS process.
        // This is the point where 'SecretString' protection ends.
        command.envs(env_map.iter().map(|(k, v)| (k, v.expose_secret())));

        // Inherit Standard IO so logs show up in the console
        command.stdout(std::process::Stdio::inherit());
        command.stderr(std::process::Stdio::inherit());

        // Spawn
        info!(cmd = ?self.cmd, "Spawning child process");
        let child = command.spawn()?;
        self.child = Some(child);

        Ok(())
    }

    pub async fn wait(&mut self) -> Result<(), ExecError> {
        let child = self.child.as_mut().ok_or(ExecError::NoChild)?;
        
        let status = child.wait().await?;
        
        if status.success() {
            Ok(())
        } else {
            Err(ExecError::from(status))
        }
    }
    
    pub async fn start(&mut self) -> anyhow::Result<()> {
        let env = self.env.resolve().await?;
        self.env_hash = Self::hash_env(&env);
        self.restart(&env).await
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

                    if let Err(e) = self.restart(&resolved).await {
                        error!("Failed to restart process: {}", e);
                    }
                } else {
                    debug!("Files changed but resolved environment is identical; skipping restart");
                }
            }
            Err(e) => {
                // Log but don't crash the watcher loop
                error!("Failed to reload environment: {}", e);
            }
        }
        Ok(())
    }
}
