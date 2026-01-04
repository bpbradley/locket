//! Filesystem path normalization and security utilities.
//!
//! This module provides the [`PathExt`] trait, which standardizes how `locket` handles
//! file paths.
//!
//! Using these utilities prevents path traversal vulnerabilities when handling user inputs.

use crate::secrets::SecretError;
use std::ops::Deref;
use std::path::{Component, Path, PathBuf};
use std::str::FromStr;

/// A path that is guaranteed to be absolute and normalized.
///
/// This type enforces that the contained path is anchored to a root (absolute)
/// and is free of relative components like `.` or `..` (lexically cleaned).
///
/// This type does not verify existence on disk. Use [`CanonicalPath`] for that.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AbsolutePath(PathBuf);

impl AbsolutePath {
    pub fn into_inner(self) -> PathBuf {
        self.0
    }
    pub fn new(path: impl AsRef<Path>) -> Self {
        Self(path.as_ref().absolute())
    }
    pub fn canonicalize(&self) -> Result<CanonicalPath, SecretError> {
        CanonicalPath::try_new(&self.0)
    }
    pub fn as_path(&self) -> &Path {
        &self.0
    }
    pub fn parent(&self) -> Option<AbsolutePath> {
        self.0.parent().map(AbsolutePath::new)
    }
    pub fn join(&self, path: impl AsRef<Path>) -> Self {
        Self::new(self.0.join(path))
    }
}

/// A path that is guaranteed to be canonical, absolute, and existing on disk.
///
/// Constructing this type performs filesystem I/O to validate existence
/// and resolve links. It therefore has a performance cost compared to [`AbsolutePath`].
/// But this should be the preferred type for source paths which must exist.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CanonicalPath(PathBuf);

impl CanonicalPath {
    pub fn into_inner(self) -> PathBuf {
        self.0
    }
    pub fn try_new(path: impl AsRef<Path>) -> Result<Self, SecretError> {
        Ok(Self(path.as_ref().canon()?))
    }
    pub fn as_path(&self) -> &Path {
        &self.0
    }
    pub fn join(&self, path: impl AsRef<Path>) -> AbsolutePath {
        AbsolutePath::new(self.0.join(path))
    }
    pub fn parent(&self) -> Option<AbsolutePath> {
        self.0.parent().map(AbsolutePath::new)
    }
}

impl From<CanonicalPath> for AbsolutePath {
    fn from(canon: CanonicalPath) -> Self {
        Self(canon.0)
    }
}


impl From<PathBuf> for AbsolutePath {
    fn from(p: PathBuf) -> Self {
        Self::new(p)
    }
}

impl From<&Path> for AbsolutePath {
    fn from(p: &Path) -> Self {
        Self::new(p)
    }
}

impl From<&PathBuf> for AbsolutePath {
    fn from(p: &PathBuf) -> Self {
        Self::new(p)
    }
}

impl TryFrom<PathBuf> for CanonicalPath {
    type Error = SecretError;

    fn try_from(p: PathBuf) -> Result<Self, Self::Error> {
        CanonicalPath::try_new(&p)
    }
}

impl TryFrom<&Path> for CanonicalPath {
    type Error = SecretError;

    fn try_from(p: &Path) -> Result<Self, Self::Error> {
        CanonicalPath::try_new(p)
    }
}

impl TryFrom<&PathBuf> for CanonicalPath {
    type Error = SecretError;

    fn try_from(p: &PathBuf) -> Result<Self, Self::Error> {
        CanonicalPath::try_new(p)
    }
}

/// Extension trait for `Path` to provide robust normalization and security checks.
trait PathExt {
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
    src: CanonicalPath,
    dst: AbsolutePath,
}

impl PathMapping {
    /// Creates a new mapping where the source MUST exist.
    ///
    /// This calls `canon()` on the source, ensuring it is a valid path on disk.
    /// The destination does not need to exist, so it is only made absolute.
    pub fn try_new(src: CanonicalPath, dst: AbsolutePath) -> Result<Self, SecretError> {
        Ok(Self { src, dst })
    }
    pub fn src(&self) -> &CanonicalPath {
        &self.src
    }
    pub fn dst(&self) -> &AbsolutePath {
        &self.dst
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
        PathMapping::try_new(CanonicalPath::from_str(src)?, AbsolutePath::from_str(dst)?)
            .map_err(|e| format!("Failed to create PathMapping '{}': {}", src, e))
    }
}

impl Deref for AbsolutePath {
    type Target = Path;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl AsRef<Path> for AbsolutePath {
    fn as_ref(&self) -> &Path {
        &self.0
    }
}

impl std::fmt::Display for AbsolutePath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.display().fmt(f)
    }
}

impl FromStr for AbsolutePath {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(AbsolutePath(Path::new(s).absolute()))
    }
}

impl Deref for CanonicalPath {
    type Target = Path;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl AsRef<Path> for CanonicalPath {
    fn as_ref(&self) -> &Path {
        &self.0
    }
}

impl std::fmt::Display for CanonicalPath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.display().fmt(f)
    }
}

impl FromStr for CanonicalPath {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        CanonicalPath::try_new(Path::new(s)).map_err(|e| e.to_string())
    }
}

impl PartialEq<Path> for AbsolutePath {
    fn eq(&self, other: &Path) -> bool {
        self.0 == other
    }
}

impl PartialEq<PathBuf> for AbsolutePath {
    fn eq(&self, other: &PathBuf) -> bool {
        self.0 == *other
    }
}

impl PartialEq<AbsolutePath> for Path {
    fn eq(&self, other: &AbsolutePath) -> bool {
        self == other.0
    }
}

impl PartialEq<AbsolutePath> for PathBuf {
    fn eq(&self, other: &AbsolutePath) -> bool {
        *self == other.0
    }
}

impl PartialEq<Path> for CanonicalPath {
    fn eq(&self, other: &Path) -> bool {
        self.0 == other
    }
}

impl PartialEq<PathBuf> for CanonicalPath {
    fn eq(&self, other: &PathBuf) -> bool {
        self.0 == *other
    }
}

impl PartialEq<CanonicalPath> for Path {
    fn eq(&self, other: &CanonicalPath) -> bool {
        self == other.0
    }
}

impl PartialEq<CanonicalPath> for PathBuf {
    fn eq(&self, other: &CanonicalPath) -> bool {
        *self == other.0
    }
}

impl std::borrow::Borrow<Path> for AbsolutePath {
    fn borrow(&self) -> &Path {
        &self.0
    }
}

impl std::borrow::Borrow<Path> for CanonicalPath {
    fn borrow(&self) -> &Path {
        &self.0
    }
}

pub fn parse_secret_path(s: &str) -> Result<crate::secrets::Secret, String> {
    crate::secrets::Secret::from_file(s).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

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

    #[test]
    fn test_absolute_path_cleaning() {
        let p = AbsolutePath::new("a/b/../c");
        let s = p.to_string();
        assert!(!s.contains(".."), "Path should be cleaned of '..'");
        assert!(s.ends_with("c"), "Path should end with 'c'");
    }

    #[test]
    fn test_canonical_path_must_exist() {
        let tmp = tempdir().unwrap();
        let file_path = tmp.path().join("config.yaml");

        // File doesn't exist -> Error
        let res = CanonicalPath::try_new(&file_path);
        assert!(matches!(res, Err(SecretError::SourceMissing(_))));

        // File exists -> Success
        std::fs::write(&file_path, "content").unwrap();
        let res = CanonicalPath::try_new(&file_path);
        assert!(res.is_ok());

        // File is a symlink -> Resolves to real path
        let link_path = tmp.path().join("link");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&file_path, &link_path).unwrap();

        #[cfg(unix)]
        {
            let canon = CanonicalPath::try_new(&link_path).unwrap();
            // CanonicalPath should resolve the symlink to the real file
            assert_eq!(canon.into_inner(), file_path.canonicalize().unwrap());
        }
    }

    #[test]
    fn test_mapping_parse() {
        let tmp = tempdir().unwrap();
        let src = tmp.path().join("src");
        std::fs::write(&src, "").unwrap();
        let src_str = src.to_str().unwrap();

        // Valid parse
        let s = format!("{}:/dst", src_str);
        let m = PathMapping::from_str(&s).expect("should parse valid mapping");
        assert_eq!(m.src(), src.canonicalize().unwrap().as_path());
        assert_eq!(m.dst(), Path::new("/dst")); // AbsolutePath::new handles the root

        // Invalid format
        assert!(PathMapping::from_str("garbage").is_err());

        // Missing source file
        let s_missing = format!("{}_missing:/dst", src_str);
        assert!(PathMapping::from_str(&s_missing).is_err());
    }
}
