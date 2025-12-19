use super::{FsEvent, WatchHandler};
use crate::{env::EnvManager, signal::wait_for_signal};
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
use std::sync::Mutex;
use std::time::Duration;
use sysexits::ExitCode;
use thiserror::Error;
use tokio::process::Command;
use tokio::signal::unix::{SignalKind, signal};
use tokio::sync::watch;
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

/// Watch handler that manages a child process, restarting it
/// when the environment changes.
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
pub struct ProcessHandler {
    env: EnvManager,
    cmd: Vec<String>,
    env_hash: u64,
    target: Option<Pid>,
    forwarder: Option<JoinHandle<()>>,
    monitor: Option<JoinHandle<()>>,
    exit_rx: watch::Receiver<Option<ExitStatus>>,
    termios: Option<Mutex<Termios>>,
    interactive: bool,
    timeout: Duration,
}

impl ProcessHandler {
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
        ProcessHandler {
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

    pub async fn wait(&mut self) -> Result<ExitStatus, ExecError> {
        // Since we moved the child to the background task, we can't wait on it directly.
        // But we can wait on the receiver for the exit status.
        if let Some(status) = *self.exit_rx.borrow() {
            return Ok(status);
        }

        let mut rx = self.exit_rx.clone();
        loop {
            // changed() waits for the value to update
            if rx.changed().await.is_err() {
                // Sender dropped
                return Err(ExecError::NoChild);
            }

            // Check the new value
            if let Some(status) = *rx.borrow() {
                return Ok(status);
            }
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

impl Drop for ProcessHandler {
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
            debug!("ProcessHandler dropped, force killing PID {:?}", pid);
            let _ = signal::kill(pid, Signal::SIGKILL);
        }
        self.reset_tty();
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
    fn exit_notify(&self) -> BoxFuture<'static, ExitCode> {
        let mut rx = self.exit_rx.clone();
        let os_signal = wait_for_signal(self.interactive);

        let child_exit = async move {
            let _ = rx.wait_for(|val| val.is_some()).await;
            *rx.borrow()
        };

        Box::pin(async move {
            tokio::select! {
                Some(status) = child_exit => {
                    if status.success() {
                        ExitCode::Ok
                    } else {
                        // Maybe can try to better map to specific ExitCodes.
                        // For now just log the actual code and return software error.
                        if let Some(code) = status.code() {
                            tracing::error!("Child process exited with code {}", code);
                        } else {
                            tracing::error!("Child process terminated by signal");
                        }
                        ExitCode::Software
                    }
                }
                _ = os_signal => ExitCode::Ok,
            }
        })
    }

    async fn cleanup(&mut self) {
        self.stop().await;
    }
}
