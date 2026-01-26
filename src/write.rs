//! Utilities for writing files atomically with explicit permissions.
//!
//! This module provides the `FileWriter` struct, which can write data to files
//! using temporary files and atomic renames to ensure that consumers never
//! see partially written files. It also ensures that the destination directories
//! exist with the correct permissions before writing.
use crate::path::{AbsolutePath, CanonicalPath};
use clap::Args;
use locket_derive::LayeredConfig;
use nix::unistd::{Gid, Uid};
use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::io::{self, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::str::FromStr;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum WriterError {
    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    Mode(#[from] FsModeError),
}

/// Specific errors related to FsMode parsing and validation.
#[derive(Debug, Error)]
pub enum FsModeError {
    #[error("invalid octal format '{input}': {source}")]
    InvalidOctal {
        input: String,
        #[source]
        source: std::num::ParseIntError,
    },

    #[error("symbolic permission must be 9 chars (e.g. 'rwxr-xr-x'), got '{0}'")]
    InvalidSymbolicLen(String),

    #[error("invalid symbolic character '{char}' at position {pos}")]
    InvalidSymbolicChar { char: char, pos: usize },

    #[error("permission mode 0o{0:o} exceeds limit 0o7777")]
    ValueTooLarge(u32),

    #[error("invalid owner format '{0}', expected 'uid:gid'")]
    InvalidOwner(String),

    #[error("invalid uid '{input}': {source}")]
    InvalidUid {
        input: String,
        source: std::num::ParseIntError,
    },

    #[error("invalid gid '{input}': {source}")]
    InvalidGid {
        input: String,
        source: std::num::ParseIntError,
    },
}

/// Utilities for writing files atomically with explicit permissions.
#[derive(Clone, Args, Serialize, Deserialize, LayeredConfig, Debug, Default)]
#[serde(rename_all = "kebab-case")]
#[locket(try_into = "FileWriter")]
pub struct FileWriterArgs {
    /// File permission mode
    #[clap(long, env = "LOCKET_FILE_MODE")]
    #[locket(default = FsMode::new(0o600))]
    file_mode: Option<FsMode>,
    /// Directory permission mode
    #[clap(long, env = "LOCKET_DIR_MODE")]
    #[locket(default = FsMode::new(0o700))]
    dir_mode: Option<FsMode>,

    /// Owner of the file/dir
    ///
    /// Defaults to the running user/group.
    /// The running user must have write permissions on the directory to change the owner.
    #[clap(long, env = "LOCKET_FILE_OWNER")]
    #[locket(optional)]
    user: Option<FsOwner>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct FileWriter {
    file_mode: FsMode,
    dir_mode: FsMode,
    user: Option<FsOwner>,
}

impl FileWriter {
    pub fn new(file_mode: FsMode, dir_mode: FsMode) -> Self {
        Self {
            file_mode,
            dir_mode,
            user: None,
        }
    }

    pub fn with_user(mut self, user: FsOwner) -> Self {
        self.user = Some(user);
        self
    }

    pub fn get_user(&self) -> Option<&FsOwner> {
        self.user.as_ref()
    }

    /// Writes data to a temporary file and atomically swaps it into place.
    ///
    /// This ensures that consumers never see a partially written file.
    /// It also ensures the destination directory exists with the configured permissions.
    pub fn atomic_write(&self, path: &AbsolutePath, bytes: &[u8]) -> Result<(), WriterError> {
        let parent = self.prepare(path)?;

        let mut tmp = tempfile::Builder::new()
            .prefix(".tmp.")
            .permissions(fs::Permissions::from_mode(self.file_mode.into()))
            .tempfile_in(parent)?;

        tmp.write_all(bytes)?;

        self.apply_file_owner(tmp.as_file())?;

        tmp.as_file().sync_all()?;
        tmp.persist(path).map_err(|e| e.error)?;

        self.sync_dir(parent)?;

        Ok(())
    }

    /// Streams data from source to destination using a temporary file for atomicity.
    pub fn atomic_copy(&self, from: &CanonicalPath, to: &AbsolutePath) -> Result<(), WriterError> {
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
    pub fn atomic_move(&self, from: &CanonicalPath, to: &AbsolutePath) -> Result<(), WriterError> {
        let parent = self.prepare(to)?;

        fs::rename(from, to)?;

        self.sync_dir(parent)?;
        Ok(())
    }

    fn apply_path_owner(&self, path: &Path) -> Result<(), WriterError> {
        if let Some(user) = self.user {
            let (u, g) = user.as_nix();
            tracing::debug!("Changing owner of {:?} to {:?}:{:?}", path, u, g);
            nix::unistd::chown(path, Some(u), Some(g)).map_err(std::io::Error::from)?;
        }
        Ok(())
    }

    fn apply_file_owner(&self, file: &std::fs::File) -> Result<(), WriterError> {
        if let Some(user) = self.user {
            let (u, g) = user.as_nix();
            tracing::debug!("Changing owner of {:?} to {:?}:{:?}", file, u, g);
            nix::unistd::fchown(file, Some(u), Some(g)).map_err(std::io::Error::from)?;
        }
        Ok(())
    }

    pub fn create_temp_for(
        &self,
        dst: &AbsolutePath,
    ) -> Result<tempfile::NamedTempFile, WriterError> {
        let parent = self.prepare(dst)?;
        let temp = tempfile::Builder::new()
            .prefix(".tmp.")
            .permissions(fs::Permissions::from_mode(self.file_mode.into()))
            .tempfile_in(parent)?;

        Ok(temp)
    }

    /// Ensures parent directory exists and applies configured directory permissions.
    fn prepare<'a>(&self, path: &'a Path) -> Result<&'a Path, WriterError> {
        let parent = path
            .parent()
            .ok_or_else(|| std::io::Error::other("path has no parent"))?;

        if !parent.exists() {
            fs::create_dir_all(parent)?;

            let perm = fs::Permissions::from_mode(self.dir_mode.into());
            fs::set_permissions(parent, perm)?;
            self.apply_path_owner(parent)?;
        }
        Ok(parent)
    }

    fn sync_dir(&self, dir: &Path) -> Result<(), WriterError> {
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
            user: None,
        }
    }
}

impl std::fmt::Debug for FileWriter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FileWriter")
            .field("file_mode", &format_args!("0o{:?}", self.file_mode))
            .field("dir_mode", &format_args!("0o{:?}", self.dir_mode))
            .field("owner", &self.user)
            .finish()
    }
}

/// Wrapper for filesystem permission bits (e.g., 0o600).
///
/// This ensures that permission values are validated (must be <= 0o7777)
/// and correctly interpreted as octal.
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[serde(try_from = "String")]
pub struct FsMode(u32);

impl TryFrom<String> for FsMode {
    type Error = FsModeError;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        s.parse()
    }
}

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
    pub fn try_new(mode: u32) -> Result<Self, FsModeError> {
        if mode > 0o7777 {
            return Err(FsModeError::ValueTooLarge(mode));
        }
        Ok(Self(mode))
    }

    fn from_symbolic(s: &str) -> Result<Self, FsModeError> {
        if s.len() != 9 {
            return Err(FsModeError::InvalidSymbolicLen(s.to_string()));
        }

        let mut mode = 0u32;
        let chars: Vec<char> = s.chars().collect();

        // Iterate over user (0), group (1), other (2)
        for (i, chunk) in chars.chunks(3).enumerate() {
            let mut bits = 0;

            let check =
                |c: char, target: char, val: u32, offset: usize| -> Result<u32, FsModeError> {
                    match c {
                        x if x == target => Ok(val),
                        '-' => Ok(0),
                        x => Err(FsModeError::InvalidSymbolicChar {
                            char: x,
                            pos: offset,
                        }),
                    }
                };

            bits |= check(chunk[0], 'r', 4, i * 3)?;
            bits |= check(chunk[1], 'w', 2, i * 3 + 1)?;
            bits |= check(chunk[2], 'x', 1, i * 3 + 2)?;

            // Shift: User(6), Group(3), Other(0)
            mode |= bits << ((2 - i) * 3);
        }

        Ok(Self(mode))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[serde(try_from = "String")]
pub struct FsOwner {
    pub uid: u32,
    pub gid: u32,
}

impl FsOwner {
    pub fn new(uid: u32, gid: u32) -> Self {
        Self { uid, gid }
    }

    pub fn as_nix(&self) -> (Uid, Gid) {
        (Uid::from_raw(self.uid), Gid::from_raw(self.gid))
    }
}

impl TryFrom<String> for FsOwner {
    type Error = FsModeError;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        s.parse()
    }
}

impl FromStr for FsOwner {
    type Err = FsModeError;

    fn from_str(s: &str) -> Result<Self, FsModeError> {
        let parts: Vec<&str> = s.split(':').collect();
        if parts.len() != 2 {
            return Err(FsModeError::InvalidOwner(s.to_string()));
        }

        let uid = parts[0].parse().map_err(|e| FsModeError::InvalidUid {
            input: parts[0].to_string(),
            source: e,
        })?;

        let gid = parts[1].parse().map_err(|e| FsModeError::InvalidGid {
            input: parts[1].to_string(),
            source: e,
        })?;

        Ok(Self { uid, gid })
    }
}

impl std::fmt::Display for FsOwner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.uid, self.gid)
    }
}

impl Serialize for FsMode {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.collect_str(self)
    }
}

impl FromStr for FsMode {
    type Err = FsModeError;

    fn from_str(s: &str) -> Result<Self, FsModeError> {
        // symbolic if it contains letters or dash
        if s.chars().any(|c| matches!(c, 'r' | 'w' | 'x' | '-')) {
            return Self::from_symbolic(s);
        }

        let norm = s.strip_prefix("0o").unwrap_or(s);
        let mode = u32::from_str_radix(norm, 8).map_err(|e| FsModeError::InvalidOctal {
            input: s.to_string(),
            source: e,
        })?;

        Self::try_new(mode)
    }
}

impl std::fmt::Debug for FsMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "0{:o}", self.0)
    }
}

impl std::fmt::Display for FsMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "0{:o}", self.0)
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
    use nix::unistd::{getgid, getuid};

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
    fn test_symbolic_parsing() {
        // rw------- -> 600
        let mode: FsMode = "rw-------".parse().unwrap();
        assert_eq!(u32::from(mode), 0o600);

        // rwxr-xr-x -> 755
        let mode: FsMode = "rwxr-xr-x".parse().unwrap();
        assert_eq!(u32::from(mode), 0o755);
        // --x--x--x -> 111
        let mode: FsMode = "--x--x--x".parse().unwrap();
        assert_eq!(u32::from(mode), 0o111);
    }

    #[test]
    fn test_symbolic_parsing_errors() {
        assert!(matches!(
            "rwx".parse::<FsMode>(),
            Err(FsModeError::InvalidSymbolicLen(_))
        ));
        assert!(matches!(
            "rw-r--r-X".parse::<FsMode>(),
            Err(FsModeError::InvalidSymbolicChar { char: 'X', .. })
        ));
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
        let tmp_dir = tempfile::tempdir().unwrap();
        let tmp = AbsolutePath::new(tmp_dir.path());
        let src = tmp.join("src");
        let dst = tmp.join("dst");

        fs::write(&src, b"content").unwrap();

        let src = src.canonicalize().expect("src must exist");

        let writer = FileWriter::default();
        writer.atomic_copy(&src, &dst).unwrap();

        let content = fs::read(dst).unwrap();
        assert_eq!(content, b"content");
    }

    #[test]
    fn test_fs_owner_parsing() {
        let owner: FsOwner = "1000:1000".parse().expect("should parse");
        assert_eq!(owner.uid, 1000);
        assert_eq!(owner.gid, 1000);

        assert!(matches!(
            "1000".parse::<FsOwner>(),
            Err(FsModeError::InvalidOwner(_))
        ));

        assert!(matches!(
            "abc:1000".parse::<FsOwner>(),
            Err(FsModeError::InvalidUid { .. })
        ));
    }

    #[test]
    fn test_writer_applies_ownership() {
        let tmp = tempfile::tempdir().unwrap();
        let path = AbsolutePath::new(tmp.path().join("owned_file"));

        let current_uid = getuid().as_raw();
        let current_gid = getgid().as_raw();

        let self_owner = FsOwner::new(current_uid, current_gid);
        let writer = FileWriter::default().with_user(self_owner);

        writer
            .atomic_write(&path, b"test")
            .expect("Should succeed chowning to self");

        if current_uid != 0 {
            let other_owner = FsOwner::new(current_uid + 1, current_gid);
            let writer = FileWriter::default().with_user(other_owner);

            let err = writer.atomic_write(&path, b"test").unwrap_err();

            match err {
                WriterError::Io(e) => assert_eq!(e.kind(), std::io::ErrorKind::PermissionDenied),
                _ => panic!("Expected IO error"),
            }
        }
    }

    #[test]
    fn test_writer_recursive_directory_ownership() {
        let tmp = tempfile::tempdir().unwrap();
        let dst = AbsolutePath::new(tmp.path().join("subdir/file"));

        let current_uid = getuid().as_raw();
        let current_gid = getgid().as_raw();

        if current_uid != 0 {
            let bad_owner = FsOwner::new(current_uid + 1, current_gid);
            let writer = FileWriter::default().with_user(bad_owner);

            let err = writer.atomic_write(&dst, b"data").unwrap_err();

            match err {
                WriterError::Io(e) => assert_eq!(e.kind(), std::io::ErrorKind::PermissionDenied),
                _ => panic!("Expected PermissionDenied trying to chown directory"),
            }
        }
    }
}
