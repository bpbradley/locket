//! Optionally embeds the `locket-op-bridge` binary into locket.
//!
//! Embedding happens only when the `op` feature is enabled AND an env
//! var points at a prebuilt bridge for the compilation target
//!
//! The per-target form exists because dist builds several targets in
//! one CI job. With neither set (crates.io installs, plain dev builds)
//! this script is inert and locket falls back to runtime discovery.

use sha2::{Digest, Sha256};
use std::path::PathBuf;

fn main() {
    println!("cargo::rustc-check-cfg=cfg(locket_embed_op_bridge)");
    println!("cargo::rerun-if-env-changed=LOCKET_OP_BRIDGE_BIN");
    let target = std::env::var("TARGET").expect("cargo sets TARGET");
    let target_var = format!(
        "LOCKET_OP_BRIDGE_BIN_{}",
        target.to_uppercase().replace(['-', '.'], "_")
    );
    println!("cargo::rerun-if-env-changed={target_var}");

    if std::env::var_os("CARGO_FEATURE_OP").is_none() {
        return;
    }
    let Some(bridge_path) =
        std::env::var_os(&target_var).or_else(|| std::env::var_os("LOCKET_OP_BRIDGE_BIN"))
    else {
        return;
    };
    let bridge_path = PathBuf::from(bridge_path);
    println!("cargo::rerun-if-changed={}", bridge_path.display());

    let bytes = std::fs::read(&bridge_path).unwrap_or_else(|e| {
        panic!(
            "op bridge embedding requested but {} is unreadable: {e}",
            bridge_path.display()
        )
    });
    assert!(
        !bytes.is_empty(),
        "op bridge embedding requested but {} is empty",
        bridge_path.display()
    );

    let out_dir = PathBuf::from(std::env::var_os("OUT_DIR").expect("cargo sets OUT_DIR"));
    std::fs::write(out_dir.join("locket-op-bridge.bin"), &bytes).expect("OUT_DIR must be writable");
    let sha256_hex: String = Sha256::digest(&bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    std::fs::write(out_dir.join("locket-op-bridge.sha256"), sha256_hex)
        .expect("OUT_DIR must be writable");

    println!("cargo::rustc-cfg=locket_embed_op_bridge");
}
