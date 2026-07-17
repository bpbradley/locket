//! Everything required to run and talk to the bundled
//! `locket-op-bridge` process: executable discovery, process spawning,
//! and the JSON pipe protocol.
//!
//! This module is deliberately self-contained behind the [`Bridge`]
//! facade so that a future backend (e.g. an official 1Password Rust
//! SDK) can replace it wholesale without touching provider logic.

mod discover;
#[cfg(locket_embed_op_bridge)]
mod embedded;
mod protocol;
mod transport;

pub(super) use protocol::ResolveResult;

use crate::path::AbsolutePath;
use crate::provider::ProviderError;
use secrecy::SecretString;
use std::collections::HashMap;

/// A running, authenticated bridge process.
pub(super) struct Bridge {
    // Declared before the child so the transport (and its reader task)
    // shuts down first; kill_on_drop then reaps the process, which also
    // exits on its own once its stdin pipe closes.
    transport: transport::BridgeTransport,
    _child: Option<tokio::process::Child>,
}

impl Bridge {
    /// Discover the bridge executable, spawn it, and complete the init
    /// handshake with the given service account token.
    pub(super) async fn connect(
        explicit: Option<&AbsolutePath>,
        token: &SecretString,
    ) -> Result<Self, ProviderError> {
        let exec = discover::BridgeExec::discover(explicit)?;
        let (child, transport) = transport::BridgeTransport::spawn(exec.command())?;
        let info = transport.init(token).await?;
        tracing::debug!(bridge_version = %info.bridge_version, "locket-op-bridge ready");
        Ok(Self {
            transport,
            _child: Some(child),
        })
    }

    /// Resolve a batch of raw `op://` references in one authenticated
    /// round trip. Results are keyed by the exact request reference.
    pub(super) async fn resolve(
        &self,
        refs: &[&str],
    ) -> Result<HashMap<String, ResolveResult>, ProviderError> {
        self.transport.resolve(refs).await
    }

    /// A bridge speaking over arbitrary pipes instead of a child process
    #[cfg(test)]
    pub(super) fn from_pipes(
        reader: impl tokio::io::AsyncRead + Send + Unpin + 'static,
        writer: impl tokio::io::AsyncWrite + Send + Unpin + 'static,
    ) -> Self {
        Self {
            transport: transport::BridgeTransport::new(reader, writer),
            _child: None,
        }
    }
}
