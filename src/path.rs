//! Filesystem path normalization and security utilities.
//!
//! This module provides the [`PathExt`] trait, which standardizes how `locket` handles
//! file paths.
//!
//! Using these utilities prevents path traversal vulnerabilities when handling user inputs.

use crate::secrets::SecretError;
use std::path::{Component, Path, PathBuf};
use std::str::FromStr;

/// Extension trait for `Path` to provide robust normalization and security checks.
pub trait PathExt {
    /// Logically cleans the path by resolving `.` and `..` components.
    ///
    /// This is a lexical operation. It does not touch the filesystem,
    /// does not resolve symlinks, and does not verify existence.
    fn clean(&self) -> PathBuf;
    /// Converts the path to an absolute path anchored to the current working directory.
    ///
    /// This method attempts to use `std::path::absolute` but falls back to `clean()`
    /// if the current directory cannot be determined.
    fn absolute(&self) -> PathBuf;
    /// Canonicalizes the path on the filesystem.
    ///
    /// This operation hits the disk. It resolves all symlinks
    /// and strictly requires that the file exists. This is the preferred method
    /// for validating user input.
    ///
    /// # Errors
    /// Returns `SecretError::SourceMissing` if the path does not exist.
    fn canon(&self) -> Result<PathBuf, SecretError>;
}

impl PathExt for Path {
    fn clean(&self) -> PathBuf {
        let mut components = self.components().peekable();
        let mut ret = if let Some(c @ Component::Prefix(..)) = components.peek().cloned() {
            components.next();
            PathBuf::from(c.as_os_str())
        } else {
            PathBuf::new()
        };

        for component in components {
            match component {
                Component::Prefix(..) => unreachable!(),
                Component::RootDir => {
                    ret.push(component.as_os_str());
                }
                Component::CurDir => {}
                Component::ParentDir => {
                    ret.pop();
                }
                Component::Normal(c) => {
                    ret.push(c);
                }
            }
        }
        ret
    }
    fn absolute(&self) -> PathBuf {
        let anchored = std::path::absolute(self).unwrap_or_else(|_| self.to_path_buf());
        anchored.clean()
    }
    fn canon(&self) -> Result<PathBuf, SecretError> {
        self.canonicalize().map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => SecretError::SourceMissing(self.to_path_buf()),
            _ => SecretError::Io(e),
        })
    }
}

/// A validated mapping of a source path to a destination path.
///
/// Used for mapping secret templates (input) to their materialized locations (output).
#[derive(Debug, Clone)]
pub struct PathMapping {
    src: PathBuf,
    dst: PathBuf,
}

impl PathMapping {
    /// Creates a new mapping with absolute paths.
    ///
    /// This does NOT verify existence.
    pub fn new(src: impl AsRef<Path>, dst: impl AsRef<Path>) -> Self {
        Self {
            src: src.as_ref().absolute(),
            dst: dst.as_ref().absolute(),
        }
    }
    /// Creates a new mapping where the source MUST exist.
    ///
    /// This calls `canon()` on the source, ensuring it is a valid path on disk.
    /// The destination does not need to exist, so it is only made absolute.
    pub fn try_new(src: impl AsRef<Path>, dst: impl AsRef<Path>) -> Result<Self, SecretError> {
        let mapping = Self {
            src: src.as_ref().canon()?,
            dst: dst.as_ref().absolute(),
        };
        Ok(mapping)
    }
    pub fn src(&self) -> &Path {
        &self.src
    }
    pub fn dst(&self) -> &Path {
        &self.dst
    }
    /// Re-resolves the source path against the filesystem.
    pub fn resolve(&mut self) -> Result<(), SecretError> {
        self.src = self.src.canon()?;
        Ok(())
    }
}

impl FromStr for PathMapping {
    type Err = String;

    /// Parse a path mapping from a string of the form "SRC:DST" or "SRC=DST".
    fn from_str(s: &str) -> Result<PathMapping, String> {
        let (src, dst) = s
            .split_once(':')
            .or_else(|| s.split_once('='))
            .ok_or_else(|| {
                format!(
                    "Invalid mapping format '{}'. Expected SRC:DST or SRC=DST",
                    s
                )
            })?;
        PathMapping::try_new(src, dst)
            .map_err(|e| format!("Failed to create PathMapping '{}': {}", src, e))
    }
}

impl Default for PathMapping {
    fn default() -> Self {
        Self::new("/templates", "/run/secrets/locket")
    }
}

pub fn parse_absolute(s: &str) -> Result<PathBuf, String> {
    Ok(Path::new(s).absolute())
}

pub fn parse_secret_path(s: &str) -> Result<crate::secrets::Secret, String> {
    crate::secrets::Secret::from_file(s).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_clean() {
        assert_eq!(Path::new("a/b/c").clean(), PathBuf::from("a/b/c"));
        assert_eq!(Path::new("a/./b/./c").clean(), PathBuf::from("a/b/c"));
    }

    #[test]
    fn test_trailing_slashes() {
        assert_eq!(Path::new("a/b/").clean(), PathBuf::from("a/b"));
        assert_eq!(
            Path::new("/tmp/secret/").clean(),
            PathBuf::from("/tmp/secret")
        );
        assert_eq!(
            Path::new("secret.yaml/").clean(),
            PathBuf::from("secret.yaml")
        );
    }

    #[test]
    fn test_parent_dir_absolute() {
        assert_eq!(Path::new("/a/b/../c").clean(), PathBuf::from("/a/c"));
        assert_eq!(Path::new("/a/b/../../c").clean(), PathBuf::from("/c"));
    }

    #[test]
    fn test_root_boundary() {
        assert_eq!(Path::new("/..").clean(), PathBuf::from("/"));
        assert_eq!(Path::new("/../a").clean(), PathBuf::from("/a"));
    }

    #[test]
    fn test_complex() {
        assert_eq!(
            Path::new("./a/b/../../c/./d/").clean(),
            PathBuf::from("c/d")
        );
    }
}
