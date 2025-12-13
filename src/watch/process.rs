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
use std::time::Duration;
use thiserror::Error;
use tokio::process::{Child, Command};
use tokio::signal::unix::{SignalKind, signal};
use tokio::task::JoinHandle;
use tracing::{debug, error, info, warn};

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
    forwarder: Option<JoinHandle<()>>,
    interactive: bool,
    timeout: Duration,
}

impl ProcessHandler {
    pub fn new(env: EnvManager, cmd: Vec<String>, interactive: bool, timeout: impl Into<Duration>) -> Self {
        ProcessHandler {
            env,
            cmd,
            env_hash: 0,
            child: None,
            forwarder: None,
            interactive,
            timeout: timeout.into(),
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

    fn spawn_forwarder(child_pid: u32) -> JoinHandle<()> {
        // Use negative PID to target the Process Group
        let pgid = Pid::from_raw(-(child_pid as i32));

        tokio::spawn(async move {
            let signals = vec![
                (SignalKind::interrupt(), Signal::SIGINT, "SIGINT"),
                (SignalKind::terminate(), Signal::SIGTERM, "SIGTERM"),
                (SignalKind::hangup(), Signal::SIGHUP, "SIGHUP"),
                (SignalKind::quit(), Signal::SIGQUIT, "SIGQUIT"),
                (SignalKind::user_defined1(), Signal::SIGUSR1, "SIGUSR1"),
                (SignalKind::user_defined2(), Signal::SIGUSR2, "SIGUSR2"),
                (SignalKind::window_change(), Signal::SIGWINCH, "SIGWINCH"),
            ];

            let (tx, mut rx) = tokio::sync::mpsc::channel(32);

            for (kind, sig, name) in signals {
                match signal(kind) {
                    Ok(mut stream) => {
                        let tx = tx.clone();
                        tokio::spawn(async move {
                            while stream.recv().await.is_some() {
                                if tx.send((sig, name)).await.is_err() {
                                    break;
                                }
                            }
                        });
                    }
                    Err(e) => tracing::warn!("failed to register listener for {}: {}", name, e),
                }
            }

            // Forwarding Loop
            while let Some((sig, name)) = rx.recv().await {
                debug!("forwarding {} to process group {}", name, pgid);
                if signal::kill(pgid, sig).is_err() {
                    break;
                }
            }
        })
    }

    async fn restart(
        &mut self,
        env_map: &HashMap<String, secrecy::SecretString>,
    ) -> anyhow::Result<()> {
        self.stop().await;

        if self.cmd.is_empty() {
            return Ok(());
        }

        let mut command = Command::new(&self.cmd[0]);
        command.args(&self.cmd[1..]);
        command.envs(env_map.iter().map(|(k, v)| (k, v.expose_secret())));
        if self.interactive {
            command.stdin(std::process::Stdio::inherit());
            command.stdout(std::process::Stdio::inherit());
            command.stderr(std::process::Stdio::inherit());
        } else {
            command.process_group(0);
            command.stdin(std::process::Stdio::null());
        }

        info!(cmd = ?self.cmd, "Spawning child process");
        let child = command.spawn()?;

        if let Some(id) = child.id() {
            self.forwarder = Some(Self::spawn_forwarder(id));
        }

        self.child = Some(child);

        Ok(())
    }

    pub async fn wait(&mut self) -> Result<(), ExecError> {
        let child = self.child.as_mut().ok_or(ExecError::NoChild)?;
        let status = child.wait().await?;

        if let Some(handle) = self.forwarder.take() {
            handle.abort();
        }

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

    pub async fn stop(&mut self) {
        if let Some(handle) = self.forwarder.take() {
            handle.abort();
        }

        if let Some(mut child) = self.child.take() {
            let pgid = if self.interactive {
                // Target the specific Process
                child.id().map(|id| Pid::from_raw(id as i32))
            } else {
                // Target the Process Group
                child.id().map(|id| Pid::from_raw(-(id as i32)))
            };

            debug!("Stopping child process group (pgid: {:?})", pgid);

            if let Some(p) = pgid
                && let Err(e) = signal::kill(p, Signal::SIGTERM)
            {
                debug!("Failed to send SIGTERM to group: {}", e);
            }

            let sleep = tokio::time::sleep(self.timeout);
            tokio::pin!(sleep);

            let mut interrupt =
                signal(SignalKind::interrupt()).expect("failed to install interrupt handler");

            tokio::select! {
                res = child.wait() => {
                    match res {
                        Ok(status) => debug!("Child exited gracefully with {}", status),
                        Err(e) => error!("Error waiting for child: {}", e),
                    }
                }
                _ = &mut sleep => {
                    warn!("Child timed out after {:?}, sending SIGKILL to group", self.timeout);
                    if let Some(p) = pgid {
                         let _ = signal::kill(p, Signal::SIGKILL);
                    }
                    let _ = child.wait().await;
                }
                _ = interrupt.recv() => {
                    warn!("Received Ctrl+C during shutdown, sending SIGKILL to group");
                    if let Some(p) = pgid {
                         let _ = signal::kill(p, Signal::SIGKILL);
                    }
                    let _ = child.wait().await;
                }
            }
        }
    }
}

impl Drop for ProcessHandler {
    fn drop(&mut self) {
        // Abort the forwarder
        if let Some(handle) = self.forwarder.take() {
            handle.abort();
        }

        // Clean up the child process
        if let Some(mut child) = self.child.take() {
            match child.try_wait() {
                Ok(Some(_)) => {} // Already done
                _ => {
                    debug!("ProcessHandler dropped, killing child process");
                    let _ = child.start_kill();
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
