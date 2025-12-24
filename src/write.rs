//! Utilities for writing files atomically with explicit permissions.
//!
//! This module provides the `FileWriter` struct, which can write data to files
//! using temporary files and atomic renames to ensure that consumers never
//! see partially written files. It also ensures that the destination directories
//! exist with the correct permissions before writing.
use crate::path::{AbsolutePath, CanonicalPath};
use clap::Args;
use std::fs::{self, File};
use std::io::{self, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

/// Utilities for writing files atomically with explicit permissions.
#[derive(Clone, Args)]
pub struct FileWriter {
    /// File permission mode
    #[clap(long, env = "LOCKET_FILE_MODE", default_value = "600")]
    file_mode: FsMode,
    /// Directory permission mode
    #[clap(long, env = "LOCKET_DIR_MODE", default_value = "700")]
    dir_mode: FsMode,
}

impl FileWriter {
    pub fn new(file_mode: FsMode, dir_mode: FsMode) -> Self {
        Self {
            file_mode,
            dir_mode,
        }
    }

    /// Writes data to a temporary file and atomically swaps it into place.
    ///
    /// This ensures that consumers never see a partially written file.
    /// It also ensures the destination directory exists with the configured permissions.
    pub fn atomic_write(&self, path: &AbsolutePath, bytes: &[u8]) -> io::Result<()> {
        let parent = self.prepare(path)?;

        let mut tmp = tempfile::Builder::new()
            .prefix(".tmp.")
            .permissions(fs::Permissions::from_mode(self.file_mode.into()))
            .tempfile_in(parent)?;

        tmp.write_all(bytes)?;
        tmp.as_file().sync_all()?;

        // Atomic Swap
        // If it fails, the temp file is automatically cleaned up by the destructor.
        tmp.persist(path).map_err(|e| e.error)?;

        self.sync_dir(parent)?;

        Ok(())
    }

    /// Streams data from source to destination using a temporary file for atomicity.
    pub fn atomic_copy(&self, from: &CanonicalPath, to: &AbsolutePath) -> io::Result<()> {
        let parent = self.prepare(to)?;
        let mut source = File::open(from)?;

        let mut tmp = tempfile::Builder::new()
            .prefix(".tmp.")
            .permissions(fs::Permissions::from_mode(self.file_mode.into()))
            .tempfile_in(parent)?;

        io::copy(&mut source, &mut tmp)?;
        tmp.as_file().sync_all()?;

        tmp.persist(to).map_err(|e| e.error)?;
        self.sync_dir(parent)?;

        Ok(())
    }

    /// Renames a file within the filesystem.
    /// Note: This cannot change file permissions easily without a race condition,
    /// so we assume the source file already has the desired permissions
    /// or we rely on the directory permissions to restrict access.
    pub fn atomic_move(&self, from: &CanonicalPath, to: &AbsolutePath) -> io::Result<()> {
        let parent = self.prepare(to)?;

        fs::rename(from, to)?;

        self.sync_dir(parent)?;
        Ok(())
    }

    pub fn create_temp_for(&self, dst: &AbsolutePath) -> io::Result<tempfile::NamedTempFile> {
        let parent = self.prepare(dst)?;
        let temp = tempfile::Builder::new()
            .prefix(".tmp.")
            .permissions(fs::Permissions::from_mode(self.file_mode.into()))
            .tempfile_in(parent)?;

        Ok(temp)
    }

    /// Ensures parent directory exists and applies configured directory permissions.
    fn prepare<'a>(&self, path: &'a Path) -> io::Result<&'a Path> {
        let parent = path
            .parent()
            .ok_or_else(|| io::Error::other("path has no parent"))?;

        if !parent.exists() {
            fs::create_dir_all(parent)?;

            let perm = fs::Permissions::from_mode(self.dir_mode.into());
            fs::set_permissions(parent, perm)?;
        }
        Ok(parent)
    }

    fn sync_dir(&self, dir: &Path) -> io::Result<()> {
        let file = File::open(dir)?;
        file.sync_all()?;
        Ok(())
    }
}

impl Default for FileWriter {
    fn default() -> Self {
        Self {
            file_mode: FsMode::new(0o600),
            dir_mode: FsMode::new(0o700),
        }
    }
}

impl std::fmt::Debug for FileWriter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FileWriter")
            .field("file_mode", &format_args!("0o{:?}", self.file_mode))
            .field("dir_mode", &format_args!("0o{:?}", self.dir_mode))
            .finish()
    }
}

/// Wrapper for filesystem permission bits (e.g., 0o600).
///
/// This ensures that permission values are validated (must be <= 0o7777)
/// and correctly interpreted as octal.
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FsMode(u32);

impl FsMode {
    /// Creates a new `FsMode` from a `u32` bitmask.
    ///
    /// # Panics
    /// Panics if `mode` > 0o7777 (invalid permission bits).
    /// This check happens at compile-time if used in a `const` or `static`.
    pub const fn new(mode: u32) -> Self {
        if mode > 0o7777 {
            // Static string panic is supported in const fn
            panic!("FsMode: value exceeds 0o7777");
        }
        Self(mode)
    }

    /// Tries to create a new `FsMode`, returning an error if invalid.
    pub fn try_new(mode: u32) -> Result<Self, String> {
        if mode > 0o7777 {
            return Err(format!("Permission mode '0o{:o}' is too large", mode));
        }
        Ok(Self(mode))
    }
}

impl std::str::FromStr for FsMode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let norm = s.strip_prefix("0o").unwrap_or(s);

        let mode = u32::from_str_radix(norm, 8)
            .map_err(|e| format!("Invalid octal permission format '{}': {}", s, e))?;

        Self::try_new(mode)
    }
}

impl std::fmt::Debug for FsMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "0o{:o}", self.0)
    }
}

impl std::fmt::Display for FsMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "0o{:o}", self.0)
    }
}

impl From<FsMode> for u32 {
    fn from(mode: FsMode) -> u32 {
        mode.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[derive(Debug, Parser)]
    struct TestParser {
        #[arg(long)]
        mode: FsMode,
    }

    #[test]
    fn test_mode_const_validation() {
        const VALID: FsMode = FsMode::new(0o755);
        assert_eq!(Into::<u32>::into(VALID), 0o755);
    }

    #[test]
    #[should_panic(expected = "FsMode: value exceeds 0o7777")]
    fn test_mode_runtime_panic() {
        let invalid = 0o10000;
        let _ = FsMode::new(invalid);
    }

    #[test]
    fn test_permission_parsing() {
        let opts = TestParser::try_parse_from(["test", "--mode", "600"]).unwrap();
        assert_eq!(u32::from(opts.mode), 0o600); // 0o600 octal

        let opts = TestParser::try_parse_from(["test", "--mode", "0755"]).unwrap();
        assert_eq!(u32::from(opts.mode), 0o755);

        let opts = TestParser::try_parse_from(["test", "--mode", "0o644"]).unwrap();
        assert_eq!(u32::from(opts.mode), 0o644);
    }

    #[test]
    fn test_permission_parsing_errors() {
        assert!(TestParser::try_parse_from(["test", "--mode", "999"]).is_err());
        assert!(TestParser::try_parse_from(["test", "--mode", "abc"]).is_err());
        assert!(TestParser::try_parse_from(["test", "--mode", "0o70000"]).is_err());
    }

    #[test]
    fn test_permissions_are_applied() {
        let tmp = tempfile::tempdir().unwrap();
        let output = tmp.path().join("secure_file");

        let writer = FileWriter::default();

        writer
            .atomic_write(&AbsolutePath::new(&output), b"data")
            .unwrap();

        let meta = fs::metadata(&output).unwrap();
        let mode = meta.permissions().mode();

        // Mask 0o777 to ignore file type bits
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn test_atomic_copy_streaming() {
        let tmp = tempfile::tempdir().unwrap();
        let src = AbsolutePath::new(tmp.path().join("src"));
        let dst = AbsolutePath::new(tmp.path().join("dst"));

        fs::write(&src, b"content").unwrap();

        let src = src.canonicalize().expect("src must exist");

        let writer = FileWriter::default();
        writer.atomic_copy(&src, &dst).unwrap();

        let content = fs::read(dst).unwrap();
        assert_eq!(content, b"content");
    }
}
