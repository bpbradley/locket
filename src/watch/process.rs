use super::{FsEvent, WatchHandler};
use crate::env::EnvManager;
use async_trait::async_trait;
use nix::sys::signal::{self, Signal};
use nix::unistd::Pid;
use secrecy::ExposeSecret;
use std::collections::{HashMap, hash_map::DefaultHasher};
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::process::ExitStatus;
use thiserror::Error;
use tokio::process::{Child, Command};
use tokio::signal::unix::{SignalKind, signal};
use tracing::{debug, error, info};

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

    async fn restart(
        &mut self,
        env_map: &HashMap<String, secrecy::SecretString>,
    ) -> anyhow::Result<()> {
        // Try to gracefully stop existing process (if it exists)
        self.stop().await;

        if self.cmd.is_empty() {
            return Ok(());
        }

        // Prepare new command
        let mut command = Command::new(&self.cmd[0]);
        command.args(&self.cmd[1..]);
        command.envs(env_map.iter().map(|(k, v)| (k, v.expose_secret())));
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
        let child_id = child.id().ok_or(ExecError::NoChild)? as i32;
        let pid = Pid::from_raw(child_id);

        let signals = vec![
            (SignalKind::interrupt(), Signal::SIGINT, "SIGINT"),
            (SignalKind::terminate(), Signal::SIGTERM, "SIGTERM"),
            (SignalKind::hangup(), Signal::SIGHUP, "SIGHUP"),
            (SignalKind::quit(), Signal::SIGQUIT, "SIGQUIT"),
            (SignalKind::user_defined1(), Signal::SIGUSR1, "SIGUSR1"),
            (SignalKind::user_defined2(), Signal::SIGUSR2, "SIGUSR2"),
            (SignalKind::window_change(), Signal::SIGWINCH, "SIGWINCH"),
        ];

        // Create a channel to unify signal handling
        let (tx, mut rx) = tokio::sync::mpsc::channel(32);

        // Register listeners and spawn forwarders
        for (kind, sig, name) in signals {
            // Register the listener with the OS
            let mut stream = match signal(kind) {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!("failed to register listener for {}: {}", name, e);
                    continue;
                }
            };

            let tx = tx.clone();
            // Spawn a task to bridge the OS signal to this channel
            tokio::spawn(async move {
                // Keep forwarding until the channel closes (main loop exits)
                while stream.recv().await.is_some() {
                    if tx.send((sig, name)).await.is_err() {
                        break;
                    }
                }
            });
        }

        loop {
            tokio::select! {
                // Child Exited
                status = child.wait() => {
                    let status = status?;
                    return if status.success() {
                        Ok(())
                    } else {
                        Err(ExecError::from(status))
                    };
                }

                // The 'Some' match handles the actual event.
                // If None, it means all senders dropped
                Some((sig, name)) = rx.recv() => {
                    debug!("received {}, forwarding to child", name);
                    // Use the helper to send the signal safely
                    if let Err(e) = signal::kill(pid, sig) {
                         debug!("failed to forward signal {}: {}", name, e);
                    }
                }
            }
        }
    }

    pub async fn start(&mut self) -> anyhow::Result<()> {
        let env = self.env.resolve().await?;
        self.env_hash = Self::hash_env(&env);
        self.restart(&env).await
    }

    pub async fn stop(&mut self) {
        if let Some(mut child) = self.child.take() {
            debug!("Stopping child process (pid: {:?})", child.id());

            // Send SIGTERM
            if let Some(id) = child.id() {
                let pid = Pid::from_raw(id as i32);
                if let Err(e) = signal::kill(pid, Signal::SIGTERM) {
                    debug!("Failed to send SIGTERM: {}", e);
                }
            }

            // Wait with Timeout TODO make configurable
            let timeout = std::time::Duration::from_secs(5);

            match tokio::time::timeout(timeout, child.wait()).await {
                Ok(_) => {
                    debug!("Child exited gracefully");
                }
                Err(_) => {
                    tracing::warn!("Child did not exit within 5s, sending SIGKILL");
                    let _ = child.kill().await; // Force kill
                    let _ = child.wait().await; // Prevent zombie
                }
            }
        }
    }
}

impl Drop for ProcessHandler {
    fn drop(&mut self) {
        // If a child is still running when the handler is dropped, ensure it doesn't become a zombie or orphan.
        if let Some(mut child) = self.child.take() {
            // Check if it has already exited
            match child.try_wait() {
                Ok(Some(_)) => {} // Already done
                _ => {
                    debug!("ProcessHandler dropped, killing child process to prevent orphan");
                    let _ = child.start_kill(); // Non-blocking SIGKILL
                }
            }
        }
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
