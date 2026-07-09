//! Pipe transport to a spawned `locket-op-bridge` child.
//!
//! Requests are written to the bridge's stdin behind a mutex; a reader
//! task demuxes stdout responses back to callers by request id, so
//! overlapping `fetch_map` calls can share the one pipe. The child is
//! reaped by `kill_on_drop`, and the bridge itself exits on stdin EOF,
//! so its lifetime can never exceed locket's.

use super::protocol::{PROTOCOL_VERSION, Request, ResolveResult, Response};
use crate::provider::ProviderError;
use secrecy::{ExposeSecret, SecretString};
use std::collections::HashMap;
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStderr, Command};
use tokio::sync::{Mutex, oneshot};

const INIT_TIMEOUT: Duration = Duration::from_secs(30);
const RESOLVE_TIMEOUT: Duration = Duration::from_secs(120);

/// `None` once the bridge connection is closed.
type PendingMap = Arc<StdMutex<Option<HashMap<u64, oneshot::Sender<Response>>>>>;

#[derive(Debug)]
pub(super) struct BridgeInfo {
    pub bridge_version: String,
}

pub(super) struct BridgeTransport {
    writer: Mutex<Box<dyn AsyncWrite + Send + Unpin>>,
    pending: PendingMap,
    next_id: AtomicU64,
    reader: tokio::task::JoinHandle<()>,
}

impl BridgeTransport {
    pub(super) fn new(
        reader: impl AsyncRead + Send + Unpin + 'static,
        writer: impl AsyncWrite + Send + Unpin + 'static,
    ) -> Self {
        let pending: PendingMap = Arc::new(StdMutex::new(Some(HashMap::new())));
        let reader = tokio::spawn(demux_responses(reader, Arc::clone(&pending)));
        Self {
            writer: Mutex::new(Box::new(writer)),
            pending,
            next_id: AtomicU64::new(1),
            reader,
        }
    }

    pub(super) async fn init(&self, token: &SecretString) -> Result<BridgeInfo, ProviderError> {
        let id = self.next_id();
        let request = Request::Init {
            id,
            protocol: PROTOCOL_VERSION,
            token: token.expose_secret(),
        };
        match self.request(id, &request, INIT_TIMEOUT).await? {
            Response::InitOk {
                protocol,
                bridge_version,
                ..
            } => {
                if protocol != PROTOCOL_VERSION {
                    return Err(ProviderError::InvalidConfig(format!(
                        "op bridge speaks protocol {protocol}, locket requires {PROTOCOL_VERSION}"
                    )));
                }
                Ok(BridgeInfo { bridge_version })
            }
            Response::Error { code, message, .. } => Err(match code {
                super::protocol::ErrorCode::UnsupportedProtocol => {
                    ProviderError::InvalidConfig(message)
                }
                super::protocol::ErrorCode::Internal => ProviderError::Other(message),
                _ => ProviderError::Unauthorized(message),
            }),
            Response::ResolveOk { .. } => Err(ProviderError::Other(
                "op bridge sent an unexpected response to init".into(),
            )),
        }
    }

    pub(super) async fn resolve(
        &self,
        refs: &[&str],
    ) -> Result<HashMap<String, ResolveResult>, ProviderError> {
        let id = self.next_id();
        let request = Request::Resolve { id, refs };
        match self.request(id, &request, RESOLVE_TIMEOUT).await? {
            Response::ResolveOk { results, .. } => Ok(results),
            Response::Error { code, message, .. } => Err(code.into_provider_error(message)),
            Response::InitOk { .. } => Err(ProviderError::Other(
                "op bridge sent an unexpected response to resolve".into(),
            )),
        }
    }

    fn next_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }

    async fn request(
        &self,
        id: u64,
        request: &Request<'_>,
        timeout: Duration,
    ) -> Result<Response, ProviderError> {
        let mut line = serde_json::to_vec(request)
            .map_err(|e| ProviderError::Other(format!("failed to encode bridge request: {e}")))?;
        line.push(b'\n');

        let (tx, rx) = oneshot::channel();
        {
            let mut pending = self.pending.lock().expect("lock poisoned");
            match pending.as_mut() {
                Some(map) => map.insert(id, tx),
                None => return Err(connection_closed()),
            };
        }

        let written = {
            let mut writer = self.writer.lock().await;
            match writer.write_all(&line).await {
                Ok(()) => writer.flush().await,
                Err(e) => Err(e),
            }
        };
        if let Err(e) = written {
            self.forget(id);
            return Err(ProviderError::Io(e));
        }

        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(response)) => Ok(response),
            Ok(Err(_)) => Err(connection_closed()),
            Err(_) => {
                self.forget(id);
                Err(ProviderError::Other(format!(
                    "op bridge did not respond within {timeout:?}"
                )))
            }
        }
    }

    fn forget(&self, id: u64) {
        if let Some(map) = self.pending.lock().expect("lock poisoned").as_mut() {
            map.remove(&id);
        }
    }
}

impl Drop for BridgeTransport {
    fn drop(&mut self) {
        self.reader.abort();
    }
}

fn connection_closed() -> ProviderError {
    ProviderError::Other("op bridge connection closed".into())
}

async fn demux_responses(reader: impl AsyncRead + Unpin, pending: PendingMap) {
    let mut lines = BufReader::new(reader).lines();
    loop {
        match lines.next_line().await {
            Ok(Some(line)) if line.trim().is_empty() => {}
            Ok(Some(line)) => match serde_json::from_str::<Response>(&line) {
                Ok(response) => {
                    let id = response.id();
                    let waiter = pending
                        .lock()
                        .expect("lock poisoned")
                        .as_mut()
                        .and_then(|map| map.remove(&id));
                    match waiter {
                        Some(tx) => {
                            let _ = tx.send(response);
                        }
                        None => {
                            tracing::warn!(
                                target: "locket::op_bridge",
                                id,
                                "dropping bridge response with no matching request"
                            );
                        }
                    }
                }
                // Only the serde error is logged: the line may hold secrets.
                Err(e) => {
                    tracing::error!(
                        target: "locket::op_bridge",
                        "bridge sent a malformed response, closing connection: {e}"
                    );
                    break;
                }
            },
            Ok(None) => break,
            Err(e) => {
                tracing::error!(target: "locket::op_bridge", "bridge pipe read failed: {e}");
                break;
            }
        }
    }
    fail_pending(&pending);
}

fn fail_pending(pending: &PendingMap) {
    let Some(map) = pending.lock().expect("lock poisoned").take() else {
        return;
    };
    for (id, tx) in map {
        let _ = tx.send(Response::Error {
            id,
            code: super::protocol::ErrorCode::Internal,
            message: "op bridge connection closed".into(),
        });
    }
}

/// Spawn the bridge from a prepared command, wiring pipes and stderr
/// forwarding. The command's argv/env are the caller's business; this
/// enforces the transport invariants (piped stdio, cleared env, reaping).
pub(super) fn spawn_bridge(
    mut command: Command,
) -> Result<(Child, BridgeTransport), ProviderError> {
    command.env_clear();
    for var in ["HOME", "PATH", "TMPDIR"] {
        if let Ok(value) = std::env::var(var) {
            command.env(var, value);
        }
    }
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    let mut child = command.spawn().map_err(ProviderError::Io)?;
    let stdin = child.stdin.take().expect("bridge stdin was piped above");
    let stdout = child.stdout.take().expect("bridge stdout was piped above");
    let stderr = child.stderr.take().expect("bridge stderr was piped above");
    tokio::spawn(forward_stderr(stderr));
    Ok((child, BridgeTransport::new(stdout, stdin)))
}

async fn forward_stderr(stderr: ChildStderr) {
    let mut lines = BufReader::new(stderr).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        tracing::warn!(target: "locket::op_bridge", "{line}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use secrecy::ExposeSecret;
    use tokio::io::{ReadHalf, WriteHalf};

    type BridgeEnd = tokio::io::DuplexStream;

    fn pair() -> (BridgeTransport, ReadHalf<BridgeEnd>, WriteHalf<BridgeEnd>) {
        let (locket_end, bridge_end) = tokio::io::duplex(64 * 1024);
        let (l_read, l_write) = tokio::io::split(locket_end);
        let (b_read, b_write) = tokio::io::split(bridge_end);
        (BridgeTransport::new(l_read, l_write), b_read, b_write)
    }

    async fn read_request(
        lines: &mut tokio::io::Lines<BufReader<ReadHalf<BridgeEnd>>>,
    ) -> serde_json::Value {
        let line = lines
            .next_line()
            .await
            .expect("bridge pipe readable")
            .expect("request line present");
        serde_json::from_str(&line).expect("request is valid JSON")
    }

    async fn respond(writer: &mut WriteHalf<BridgeEnd>, response: &str) {
        writer
            .write_all(format!("{response}\n").as_bytes())
            .await
            .expect("bridge pipe writable");
    }

    #[tokio::test]
    async fn init_handshake_round_trips() {
        let (transport, b_read, mut b_write) = pair();
        let responder = tokio::spawn(async move {
            let mut lines = BufReader::new(b_read).lines();
            let req = read_request(&mut lines).await;
            let id = req["id"].as_u64().unwrap();
            respond(
                &mut b_write,
                &format!(r#"{{"type":"init-ok","id":{id},"protocol":1,"bridge_version":"9.9.9"}}"#),
            )
            .await;
            req
        });

        let info = transport
            .init(&SecretString::from("ops_test"))
            .await
            .unwrap();
        assert_eq!(info.bridge_version, "9.9.9");

        let req = responder.await.unwrap();
        assert_eq!(req["type"], "init");
        assert_eq!(req["protocol"], 1);
        assert_eq!(req["token"], "ops_test");
    }

    #[tokio::test]
    async fn init_error_maps_to_unauthorized() {
        let (transport, b_read, mut b_write) = pair();
        tokio::spawn(async move {
            let mut lines = BufReader::new(b_read).lines();
            let req = read_request(&mut lines).await;
            let id = req["id"].as_u64().unwrap();
            respond(
                &mut b_write,
                &format!(r#"{{"type":"error","id":{id},"code":"other","message":"bad token"}}"#),
            )
            .await;
        });

        let err = transport
            .init(&SecretString::from("ops_bad"))
            .await
            .unwrap_err();
        assert!(matches!(err, ProviderError::Unauthorized(m) if m == "bad token"));
    }

    #[tokio::test]
    async fn init_protocol_mismatch_maps_to_invalid_config() {
        let (transport, b_read, mut b_write) = pair();
        tokio::spawn(async move {
            let mut lines = BufReader::new(b_read).lines();
            let req = read_request(&mut lines).await;
            let id = req["id"].as_u64().unwrap();
            respond(
                &mut b_write,
                &format!(
                    r#"{{"type":"error","id":{id},"code":"unsupported_protocol","message":"v99"}}"#
                ),
            )
            .await;
        });

        let err = transport
            .init(&SecretString::from("ops_test"))
            .await
            .unwrap_err();
        assert!(matches!(err, ProviderError::InvalidConfig(_)));
    }

    #[tokio::test]
    async fn resolve_round_trips_mixed_results() {
        let (transport, b_read, mut b_write) = pair();
        tokio::spawn(async move {
            let mut lines = BufReader::new(b_read).lines();
            let req = read_request(&mut lines).await;
            let id = req["id"].as_u64().unwrap();
            assert_eq!(req["refs"][0], "op://v/i/f");
            respond(
                &mut b_write,
                &format!(
                    r#"{{"type":"resolve-ok","id":{id},"results":{{"op://v/i/f":{{"secret":"hunter2"}},"op://v/missing/f":{{"error":{{"code":"not_found","message":"nope"}}}}}}}}"#
                ),
            )
            .await;
        });

        let results = transport
            .resolve(&["op://v/i/f", "op://v/missing/f"])
            .await
            .unwrap();
        match &results["op://v/i/f"] {
            ResolveResult::Resolved { secret } => assert_eq!(secret.expose_secret(), "hunter2"),
            other => panic!("expected Resolved, got {other:?}"),
        }
        assert!(matches!(
            &results["op://v/missing/f"],
            ResolveResult::Failed { .. }
        ));
    }

    #[tokio::test]
    async fn concurrent_requests_demux_out_of_order_responses() {
        let (transport, b_read, mut b_write) = pair();
        tokio::spawn(async move {
            let mut lines = BufReader::new(b_read).lines();
            let first = read_request(&mut lines).await;
            let second = read_request(&mut lines).await;
            // Answer the second request first to prove id-based demux.
            for req in [second, first] {
                let id = req["id"].as_u64().unwrap();
                let reference = req["refs"][0].as_str().unwrap();
                respond(
                    &mut b_write,
                    &format!(
                        r#"{{"type":"resolve-ok","id":{id},"results":{{"{reference}":{{"secret":"for {reference}"}}}}}}"#
                    ),
                )
                .await;
            }
        });

        let (a, b) = tokio::join!(
            transport.resolve(&["op://v/a/f"]),
            transport.resolve(&["op://v/b/f"])
        );
        let (a, b) = (a.unwrap(), b.unwrap());
        let ResolveResult::Resolved { secret } = &a["op://v/a/f"] else {
            panic!("missing result for a");
        };
        assert_eq!(secret.expose_secret(), "for op://v/a/f");
        let ResolveResult::Resolved { secret } = &b["op://v/b/f"] else {
            panic!("missing result for b");
        };
        assert_eq!(secret.expose_secret(), "for op://v/b/f");
    }

    #[tokio::test]
    async fn eof_fails_pending_and_subsequent_requests() {
        let (transport, b_read, b_write) = pair();
        tokio::spawn(async move {
            let mut lines = BufReader::new(b_read).lines();
            let _ = read_request(&mut lines).await;
            drop(b_write);
            drop(lines);
        });

        let err = transport.resolve(&["op://v/i/f"]).await.unwrap_err();
        assert!(err.to_string().contains("connection closed"), "{err}");

        let err = transport.resolve(&["op://v/i/f"]).await.unwrap_err();
        assert!(err.to_string().contains("connection closed"), "{err}");
    }

    #[tokio::test]
    async fn malformed_bridge_line_is_fatal() {
        let (transport, b_read, mut b_write) = pair();
        tokio::spawn(async move {
            let mut lines = BufReader::new(b_read).lines();
            let _ = read_request(&mut lines).await;
            respond(&mut b_write, "this is not json").await;
            // Keep the pipe open: the transport must still fail.
            std::future::pending::<()>().await;
        });

        let err = transport.resolve(&["op://v/i/f"]).await.unwrap_err();
        assert!(err.to_string().contains("connection closed"), "{err}");
    }

    #[tokio::test]
    async fn unknown_response_id_is_ignored() {
        let (transport, b_read, mut b_write) = pair();
        tokio::spawn(async move {
            let mut lines = BufReader::new(b_read).lines();
            let req = read_request(&mut lines).await;
            let id = req["id"].as_u64().unwrap();
            respond(
                &mut b_write,
                r#"{"type":"resolve-ok","id":999,"results":{}}"#,
            )
            .await;
            respond(
                &mut b_write,
                &format!(r#"{{"type":"resolve-ok","id":{id},"results":{{}}}}"#),
            )
            .await;
        });

        let results = transport.resolve(&["op://v/i/f"]).await.unwrap();
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn request_times_out_when_bridge_hangs() {
        let (transport, _b_read, _b_write) = pair();
        let id = transport.next_id();
        let refs = ["op://v/i/f"];
        let request = Request::Resolve { id, refs: &refs };
        let err = transport
            .request(id, &request, Duration::from_millis(50))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("did not respond"), "{err}");
        assert!(
            transport
                .pending
                .lock()
                .expect("lock poisoned")
                .as_ref()
                .is_some_and(HashMap::is_empty),
            "timed out request must not leak a pending entry"
        );
    }
}
