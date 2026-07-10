#![cfg(feature = "op")]
//! Contract tests against a real `locket-op-bridge` binary.
//!
//! Ignored unless LOCKET_OP_BRIDGE_TEST_BIN points at a built bridge:
//!
//! ```sh
//! go build -C tools/op-bridge -o /tmp/locket-op-bridge .
//! LOCKET_OP_BRIDGE_TEST_BIN=/tmp/locket-op-bridge \
//!     cargo test --features testing --test op_bridge -- --ignored
//! ```
//!
//! No 1Password credentials are required: these prove the spawn,
//! handshake, and shutdown contract, not secret resolution.

use locket::path::AbsolutePath;
use locket::provider::config::op::OpConfig;
use locket::provider::{Provider, ProviderError};
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

fn bridge_bin() -> AbsolutePath {
    std::env::var("LOCKET_OP_BRIDGE_TEST_BIN")
        .expect("set LOCKET_OP_BRIDGE_TEST_BIN to run op bridge contract tests")
        .parse()
        .expect("LOCKET_OP_BRIDGE_TEST_BIN must be an absolute path")
}

/// The full public path: discovery via explicit override, spawn,
/// init handshake, and error mapping, all through Provider::build.
#[tokio::test]
#[ignore = "requires LOCKET_OP_BRIDGE_TEST_BIN"]
async fn invalid_token_fails_provider_build_as_unauthorized() {
    let cfg = OpConfig {
        op_token: "locket-invalid-test-token".parse().expect("literal token"),
        op_bridge: Some(bridge_bin()),
    };
    let err = match Provider::Op(cfg).build().await {
        Ok(_) => panic!("provider build must fail with an invalid token"),
        Err(e) => e,
    };
    assert!(matches!(err, ProviderError::Unauthorized(_)), "{err}");
}

#[tokio::test]
#[ignore = "requires LOCKET_OP_BRIDGE_TEST_BIN"]
async fn unsupported_protocol_version_is_rejected() {
    let mut child = Command::new(bridge_bin().as_path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .expect("bridge binary must spawn");

    let mut stdin = child.stdin.take().expect("stdin piped");
    stdin
        .write_all(b"{\"type\":\"init\",\"id\":1,\"protocol\":99,\"token\":\"x\"}\n")
        .await
        .unwrap();

    let mut lines = BufReader::new(child.stdout.take().expect("stdout piped")).lines();
    let line = tokio::time::timeout(Duration::from_secs(10), lines.next_line())
        .await
        .expect("bridge must respond promptly")
        .unwrap()
        .expect("bridge must write a response line");
    let response: serde_json::Value = serde_json::from_str(&line).unwrap();
    assert_eq!(response["type"], "error");
    assert_eq!(response["code"], "unsupported_protocol");
    assert_eq!(response["id"], 1);

    drop(stdin);
    let status = tokio::time::timeout(Duration::from_secs(5), child.wait())
        .await
        .expect("bridge must exit after stdin EOF")
        .unwrap();
    assert!(status.success(), "bridge must exit 0 on EOF, got {status}");
}

#[tokio::test]
#[ignore = "requires LOCKET_OP_BRIDGE_TEST_BIN"]
async fn bridge_exits_cleanly_on_immediate_eof() {
    let mut child = Command::new(bridge_bin().as_path())
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .expect("bridge binary must spawn");

    drop(child.stdin.take());
    let status = tokio::time::timeout(Duration::from_secs(5), child.wait())
        .await
        .expect("bridge must exit after stdin EOF")
        .unwrap();
    assert!(status.success(), "bridge must exit 0 on EOF, got {status}");
}
