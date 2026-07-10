//! Locates the `locket-op-bridge` executable.
//!
//! PATH is never searched and nothing is ever downloaded, so an
//! attacker cannot swap in a bridge without write access to locations
//! the user already trusts.

use crate::path::{AbsolutePath, CanonicalPath};
use crate::provider::ProviderError;
use std::path::Path;
use tokio::process::Command;

pub(super) const BRIDGE_BINARY_NAME: &str = "locket-op-bridge";

/// A spawnable bridge executable, resolved by [`BridgeExec::discover`].
#[derive(Debug)]
pub(super) enum BridgeExec {
    Path(CanonicalPath),
    #[cfg(all(target_os = "linux", any(locket_embed_op_bridge, test)))]
    Memfd(std::os::fd::OwnedFd),
}

impl BridgeExec {
    /// Resolution order: explicit override, embedded bytes, then a
    /// binary adjacent to the locket executable.
    pub(super) fn discover(explicit: Option<&AbsolutePath>) -> Result<Self, ProviderError> {
        if let Some(path) = explicit {
            return Self::explicit(path.as_path());
        }

        #[cfg(locket_embed_op_bridge)]
        {
            return Self::embedded();
        }

        #[cfg_attr(locket_embed_op_bridge, allow(unreachable_code))]
        Self::adjacent().ok_or_else(|| {
            ProviderError::InvalidConfig(format!(
                "the op provider requires the `{BRIDGE_BINARY_NAME}` binary. Install it next to \
                 the locket binary (e.g. ~/.cargo/bin/{BRIDGE_BINARY_NAME}) from \
                 https://github.com/bpbradley/locket/releases, or point --op-bridge / \
                 LOCKET_OP_BRIDGE at it."
            ))
        })
    }

    /// The command is only spawnable while `self` is alive: the memfd
    /// variant execs via `/proc/self/fd/N`, which dies with the fd.
    pub(super) fn command(&self) -> Command {
        match self {
            BridgeExec::Path(path) => Command::new(path.as_path()),
            #[cfg(all(target_os = "linux", any(locket_embed_op_bridge, test)))]
            BridgeExec::Memfd(fd) => {
                use std::os::fd::AsRawFd;
                Command::new(format!("/proc/self/fd/{}", fd.as_raw_fd()))
            }
        }
    }

    fn explicit(path: &Path) -> Result<Self, ProviderError> {
        let canonical = CanonicalPath::try_new(path).map_err(|e| {
            ProviderError::InvalidConfig(format!(
                "op bridge not found at {} (from --op-bridge / LOCKET_OP_BRIDGE): {e}",
                path.display()
            ))
        })?;
        if !canonical.as_path().is_file() {
            return Err(ProviderError::InvalidConfig(format!(
                "op bridge at {} is not a file (from --op-bridge / LOCKET_OP_BRIDGE)",
                canonical.as_path().display()
            )));
        }
        Ok(Self::Path(canonical))
    }

    fn adjacent() -> Option<Self> {
        let exe = std::env::current_exe().ok()?;
        let candidate = exe.parent()?.join(BRIDGE_BINARY_NAME);
        if !candidate.is_file() {
            return None;
        }
        CanonicalPath::try_new(&candidate).ok().map(Self::Path)
    }

    #[cfg(locket_embed_op_bridge)]
    fn embedded() -> Result<Self, ProviderError> {
        use super::embedded::{BRIDGE_BYTES, BRIDGE_SHA256};

        #[cfg(target_os = "linux")]
        match Self::memfd(BRIDGE_BYTES) {
            Ok(exec) => return Ok(exec),
            // Typically seccomp or an ancient kernel; the cache below
            // provides the same binary with one extra disk write.
            Err(e) => {
                tracing::debug!("memfd execution unavailable ({e}), extracting op bridge to cache")
            }
        }

        let path = BridgeCache::user_default()?.ensure(BRIDGE_BYTES, BRIDGE_SHA256)?;
        Ok(Self::Path(path))
    }

    /// Write the embedded bridge to memory only: no file on disk,
    /// nothing to tamper with, vanishes with the fd.
    #[cfg(all(target_os = "linux", any(locket_embed_op_bridge, test)))]
    fn memfd(bytes: &[u8]) -> std::io::Result<Self> {
        use nix::sys::memfd::{MFdFlags, memfd_create};
        use std::io::Write;

        let fd = memfd_create(c"locket-op-bridge", MFdFlags::MFD_CLOEXEC)?;
        let mut file = std::fs::File::from(fd);
        file.write_all(bytes)?;
        file.flush()?;
        Ok(Self::Memfd(file.into()))
    }
}

/// On-disk home for the embedded bridge when memfd execution is
/// unavailable (macOS, seccomp). Entries are keyed by locket version
/// plus content hash so concurrent locket versions never collide, and
/// a cache hit is re-hashed before it is trusted.
#[cfg(any(locket_embed_op_bridge, test))]
struct BridgeCache {
    root: AbsolutePath,
}

#[cfg(any(locket_embed_op_bridge, test))]
impl BridgeCache {
    #[cfg(locket_embed_op_bridge)]
    fn user_default() -> Result<Self, ProviderError> {
        // Per the XDG base directory spec, a relative XDG_CACHE_HOME
        // must be ignored rather than used.
        let base = std::env::var_os("XDG_CACHE_HOME")
            .and_then(AbsolutePath::strict)
            .or_else(|| {
                std::env::var_os("HOME")
                    .map(|home| AbsolutePath::new(std::path::PathBuf::from(home).join(".cache")))
            })
            .ok_or_else(|| {
                ProviderError::InvalidConfig(
                    "cannot place the embedded op bridge: neither XDG_CACHE_HOME nor HOME is \
                     set (set --op-bridge to use an external binary)"
                        .into(),
                )
            })?;
        Ok(Self {
            root: base.join("locket").join("op-bridge"),
        })
    }

    #[cfg(test)]
    fn at(root: &Path) -> Self {
        Self {
            root: AbsolutePath::new(root),
        }
    }

    fn ensure(&self, bytes: &[u8], sha256_hex: &str) -> Result<CanonicalPath, ProviderError> {
        use std::os::unix::fs::PermissionsExt;

        let key_len = sha256_hex.len().min(16);
        let version_dir = self.root.join(format!(
            "{}-{}",
            env!("CARGO_PKG_VERSION"),
            &sha256_hex[..key_len]
        ));
        std::fs::create_dir_all(version_dir.as_path())?;
        std::fs::set_permissions(
            version_dir.as_path(),
            std::fs::Permissions::from_mode(0o700),
        )?;

        let bridge_path = version_dir.join(BRIDGE_BINARY_NAME);
        if bridge_path.as_path().is_file() && file_sha256(bridge_path.as_path())? == sha256_hex {
            return canonical_cache_entry(&bridge_path);
        }

        // Unique temp name + rename keeps concurrent locket processes
        // from ever observing a partially written executable.
        let tmp = version_dir.join(format!(".{BRIDGE_BINARY_NAME}.{}", std::process::id()));
        std::fs::write(tmp.as_path(), bytes)?;
        std::fs::set_permissions(tmp.as_path(), std::fs::Permissions::from_mode(0o500))?;
        std::fs::rename(tmp.as_path(), bridge_path.as_path())?;
        canonical_cache_entry(&bridge_path)
    }
}

#[cfg(any(locket_embed_op_bridge, test))]
fn canonical_cache_entry(path: &AbsolutePath) -> Result<CanonicalPath, ProviderError> {
    path.canonicalize()
        .map_err(|e| ProviderError::Other(format!("op bridge cache entry unreadable: {e}")))
}

#[cfg(any(locket_embed_op_bridge, test))]
fn file_sha256(path: &Path) -> Result<String, ProviderError> {
    use sha2::{Digest, Sha256};
    let bytes = std::fs::read(path)?;
    Ok(Sha256::digest(&bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use assert_fs::TempDir;
    use assert_fs::prelude::*;
    use sha2::{Digest, Sha256};
    use std::os::unix::fs::PermissionsExt;

    fn sha_hex(bytes: &[u8]) -> String {
        Sha256::digest(bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }

    #[test]
    fn explicit_path_must_exist() {
        let err = BridgeExec::explicit(Path::new("/nonexistent/locket-op-bridge")).unwrap_err();
        assert!(matches!(err, ProviderError::InvalidConfig(_)));
    }

    #[test]
    fn explicit_path_must_be_a_file() {
        let dir = TempDir::new().unwrap();
        let err = BridgeExec::explicit(dir.path()).unwrap_err();
        assert!(matches!(err, ProviderError::InvalidConfig(_)));
    }

    #[test]
    fn explicit_path_resolves_canonically() {
        let dir = TempDir::new().unwrap();
        let bridge = dir.child(BRIDGE_BINARY_NAME);
        bridge.write_binary(b"fake").unwrap();
        let exec = BridgeExec::explicit(bridge.path()).unwrap();
        let BridgeExec::Path(path) = exec else {
            panic!("expected path exec");
        };
        assert_eq!(path.as_path(), bridge.path().canonicalize().unwrap());
    }

    #[test]
    fn cache_extracts_with_locked_down_permissions() {
        let root = TempDir::new().unwrap();
        let bytes = b"bridge contents";
        let path = BridgeCache::at(root.path())
            .ensure(bytes, &sha_hex(bytes))
            .unwrap();

        assert_eq!(std::fs::read(path.as_path()).unwrap(), bytes);
        let file_mode = std::fs::metadata(path.as_path())
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(file_mode, 0o500);
        let dir_mode = std::fs::metadata(path.as_path().parent().unwrap())
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(dir_mode, 0o700);
    }

    #[test]
    fn cache_reuses_matching_entry() {
        let root = TempDir::new().unwrap();
        let cache = BridgeCache::at(root.path());
        let bytes = b"bridge contents";
        let sha = sha_hex(bytes);
        let first = cache.ensure(bytes, &sha).unwrap();
        let first_mtime = std::fs::metadata(first.as_path())
            .unwrap()
            .modified()
            .unwrap();

        let second = cache.ensure(bytes, &sha).unwrap();
        assert_eq!(first, second);
        let second_mtime = std::fs::metadata(second.as_path())
            .unwrap()
            .modified()
            .unwrap();
        assert_eq!(first_mtime, second_mtime, "cache hit must not rewrite");
    }

    #[test]
    fn cache_replaces_corrupted_entry() {
        let root = TempDir::new().unwrap();
        let cache = BridgeCache::at(root.path());
        let bytes = b"bridge contents";
        let sha = sha_hex(bytes);
        let path = cache.ensure(bytes, &sha).unwrap();

        std::fs::set_permissions(path.as_path(), std::fs::Permissions::from_mode(0o600)).unwrap();
        std::fs::write(path.as_path(), b"tampered").unwrap();

        let restored = cache.ensure(bytes, &sha).unwrap();
        assert_eq!(restored, path);
        assert_eq!(std::fs::read(restored.as_path()).unwrap(), bytes);
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn memfd_spawns_from_memory() {
        // Any small ELF works; skip quietly on systems without /bin/true.
        let Ok(bytes) = std::fs::read("/bin/true") else {
            return;
        };
        let exec = BridgeExec::memfd(&bytes).expect("memfd_create should be available");
        let status = exec.command().status().await.unwrap();
        assert!(status.success());
    }
}
