//! Process lifecycle management and signal proxying.
//!
//! Implements the `ProcessManager`, which spawns and manages
//! a child process with a dynamically resolved environment.
//! It listens for filesystem events via the `EventHandler` trait,
//! and restarts the child process when relevant changes occur.

use crate::{
    env::{EnvError, EnvManager},
    events::{EventHandler, FsEvent, HandlerError, wait_for_signal},
};
use async_trait::async_trait;
use futures::future::BoxFuture;
use nix::sys::{
    signal::{self, Signal},
    termios::{SetArg, Termios, tcgetattr, tcsetattr},
};
use nix::unistd::Pid;
use secrecy::ExposeSecret;
use std::collections::{HashMap, hash_map::DefaultHasher};
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::process::ExitStatus;
use std::str::FromStr;
use std::sync::Mutex;
use std::time::Duration;
use thiserror::Error;
use tokio::process::Command;
use tokio::signal::unix::{SignalKind, signal};
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tracing::{debug, error, info, warn};

/// Errors specific to process lifecycle management.
#[derive(Debug, Error)]
pub enum ProcessError {
    #[error(transparent)]
    Env(#[from] EnvError),

    #[error("process I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("child process exited with status {0}")]
    Exited(ExitStatus),

    #[error("child process terminated by signal")]
    Signaled,
}

impl ProcessError {
    pub fn from_status(status: ExitStatus) -> Result<(), Self> {
        if status.success() {
            Ok(())
        } else {
            Err(Self::Exited(status))
        }
    }
}

/// Default timeout for waiting for a child process to exit gracefully after SIGTERM.
#[derive(Debug, Clone, Copy)]
pub struct ProcessTimeout(pub Duration);

/// Defaults to seconds if no unit specified, otherwise uses humantime parsing.
impl FromStr for ProcessTimeout {
    type Err = humantime::DurationError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if let Ok(s) = s.parse::<u64>() {
            return Ok(ProcessTimeout(Duration::from_secs(s)));
        }
        let duration = humantime::parse_duration(s)?;
        Ok(ProcessTimeout(duration))
    }
}

impl std::fmt::Display for ProcessTimeout {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", humantime::format_duration(self.0))
    }
}

impl From<ProcessTimeout> for Duration {
    fn from(val: ProcessTimeout) -> Self {
        val.0
    }
}

impl Default for ProcessTimeout {
    fn default() -> Self {
        ProcessTimeout(Duration::from_secs(30))
    }
}

/// Manages a child process, restarting it when the environment changes.
///
/// It spawns the child process with the resolved environment
/// from the provided `EnvManager`. When file system events are
/// detected, it checks if the environment has changed, and if so,
/// it restarts the child process with the new environment.
///
/// It also forwards signals received by the parent to the child process, or process group.
/// There are some differences in behavior when running in interactive mode, namely that
/// stdin/stdout/stderr are inherited directly, and signals are sent to the specific child process
/// rather than the process group.
pub struct ProcessManager {
    env: EnvManager,
    cmd: Vec<String>,
    env_hash: u64,
    /// The OS PID of the currently running child process.
    target: Option<Pid>,
    /// Background task that forwards OS signals to the child.
    forwarder: Option<JoinHandle<()>>,
    /// Background task that waits for the child to exit.
    monitor: Option<JoinHandle<()>>,
    /// Channel to notify the main loop when the child exits.
    exit_rx: watch::Receiver<Option<ExitStatus>>,
    /// Saved terminal state (for restoring TTY after child exit).
    termios: Option<Mutex<Termios>>,
    interactive: bool,
    timeout: Duration,
}

impl ProcessManager {
    pub fn new(
        env: EnvManager,
        cmd: Vec<String>,
        interactive: bool,
        timeout: impl Into<Duration>,
    ) -> Self {
        let (_, exit_rx) = watch::channel(None);
        let termios = if interactive {
            tcgetattr(std::io::stdin()).ok().map(Mutex::new)
        } else {
            None
        };
        ProcessManager {
            env,
            cmd,
            env_hash: 0,
            target: None,
            forwarder: None,
            monitor: None,
            exit_rx,
            termios,
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

    fn spawn_forwarder(target: Pid, interactive: bool) -> JoinHandle<()> {
        tokio::spawn(async move {
            let mut signals = vec![
                (SignalKind::interrupt(), Signal::SIGINT, "SIGINT"),
                (SignalKind::terminate(), Signal::SIGTERM, "SIGTERM"),
                (SignalKind::hangup(), Signal::SIGHUP, "SIGHUP"),
                (SignalKind::quit(), Signal::SIGQUIT, "SIGQUIT"),
                (SignalKind::user_defined1(), Signal::SIGUSR1, "SIGUSR1"),
                (SignalKind::user_defined2(), Signal::SIGUSR2, "SIGUSR2"),
                (SignalKind::window_change(), Signal::SIGWINCH, "SIGWINCH"),
            ];

            // In interactive mode, we don't forward SIGINT and SIGQUIT
            // as they are handled directly by stdin in interactive mode.
            if interactive {
                signals.retain(|(_, sig, _)| *sig != Signal::SIGINT && *sig != Signal::SIGQUIT);
            }

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
                debug!("forwarding {} to process {}", name, target);
                if signal::kill(target, sig).is_err() {
                    break;
                }
            }
        })
    }

    fn reset_tty(&self) {
        if let Some(mutex) = &self.termios
            && let Ok(guard) = mutex.lock()
        {
            let _ = tcsetattr(std::io::stdin(), SetArg::TCSANOW, &guard);
        }
    }

    async fn restart(
        &mut self,
        env_map: &HashMap<String, secrecy::SecretString>,
    ) -> Result<(), ProcessError> {
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

        // We handle killing manually via target in stop() and Drop
        command.kill_on_drop(false);

        info!(cmd = ?self.cmd, "Spawning child process");
        let mut child = command.spawn()?;

        if let Some(id) = child.id() {
            let pid = if self.interactive {
                Pid::from_raw(id as i32)
            } else {
                Pid::from_raw(-(id as i32))
            };
            self.target = Some(pid);
            self.forwarder = Some(Self::spawn_forwarder(pid, self.interactive));
        }

        let (tx, rx) = watch::channel(None);
        self.exit_rx = rx;

        self.monitor = Some(tokio::spawn(async move {
            match child.wait().await {
                Ok(status) => {
                    info!("Child process exited: {}", status);
                    let _ = tx.send(Some(status));
                }
                Err(e) => {
                    error!("Monitor failed to wait on child: {}", e);
                }
            }
        }));

        Ok(())
    }

    pub async fn start(&mut self) -> Result<(), ProcessError> {
        let env = self.env.resolve().await?;
        self.env_hash = Self::hash_env(&env);
        self.restart(&env).await?;
        Ok(())
    }

    pub async fn stop(&mut self) {
        if let Some(handle) = self.forwarder.take() {
            handle.abort();
        }

        let target = self.target.take();
        let mut monitor = self.monitor.take();

        if let Some(p) = target {
            debug!("Stopping process {:?}", p);

            if let Err(e) = signal::kill(p, Signal::SIGTERM) {
                debug!("Failed to send SIGTERM: {}", e);
            }

            if let Some(monitor_handle) = &mut monitor {
                let sleep = tokio::time::sleep(self.timeout);
                tokio::pin!(sleep);

                let mut interrupt =
                    tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())
                        .expect("failed to install interrupt handler");

                let mut finished = false;

                tokio::select! {
                    res = &mut *monitor_handle => {
                        match res {
                            Ok(_) => debug!("Child exited gracefully"),
                            Err(e) => error!("Monitor task failed: {}", e),
                        }
                        finished = true;
                    }
                    _ = &mut sleep => {
                        warn!("Child timed out after {:?}, sending SIGKILL", self.timeout);
                        let _ = signal::kill(p, Signal::SIGKILL);
                    }
                    _ = interrupt.recv() => {
                        warn!("Received Ctrl+C during shutdown, sending SIGKILL");
                        let _ = signal::kill(p, Signal::SIGKILL);
                    }
                }

                // MUST await it to ensure the zombie is reaped.
                // Since we just sent SIGKILL, this await should return immediately.
                if !finished && let Err(e) = monitor_handle.await {
                    error!("Failed to join monitor task after kill: {}", e);
                }

                self.reset_tty();
            }
        }
    }
}

impl Drop for ProcessManager {
    fn drop(&mut self) {
        if let Some(handle) = self.forwarder.take() {
            handle.abort();
        }

        if let Some(handle) = self.monitor.take() {
            handle.abort();
        }

        // Last Resort Kill
        // Since using kill_on_drop(false), aborting the monitor drops the Child
        // but DOES NOT kill the process. We must do it manually.
        if let Some(pid) = self.target {
            debug!("ProcessManager dropped, force killing PID {:?}", pid);
            let _ = signal::kill(pid, Signal::SIGKILL);
        }
        self.reset_tty();
    }
}

#[async_trait]
impl EventHandler for ProcessManager {
    fn paths(&self) -> Vec<PathBuf> {
        self.env.files()
    }

    async fn handle(&mut self, events: Vec<FsEvent>) -> Result<(), HandlerError> {
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

    fn wait(&self) -> BoxFuture<'static, Result<(), HandlerError>> {
        let mut rx = self.exit_rx.clone();
        let os_signal = wait_for_signal(self.interactive);

        let child_exit = async move {
            let _ = rx.wait_for(|val| val.is_some()).await;
            *rx.borrow()
        };

        Box::pin(async move {
            tokio::select! {
                Some(status) = child_exit => {
                    HandlerError::from_status(status)
                }
                _ = os_signal => {
                    Ok(())
                }
            }
        })
    }

    async fn cleanup(&mut self) {
        self.stop().await;
    }
}
