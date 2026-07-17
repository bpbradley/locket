//! Bridge binary embedded at build time. Only compiled when build.rs
//! found a prebuilt bridge.

pub(super) const BRIDGE_BYTES: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/locket-op-bridge.bin"));

/// Lowercase hex SHA-256 of `BRIDGE_BYTES`, used to key and verify the
/// on-disk cache when memfd execution is unavailable.
pub(super) const BRIDGE_SHA256: &str =
    include_str!(concat!(env!("OUT_DIR"), "/locket-op-bridge.sha256"));
