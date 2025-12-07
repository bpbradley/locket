use crate::secrets::SecretError;
use std::path::{Component, Path, PathBuf};
use std::str::FromStr;

/// Extension trait for Path to provide additional functionality
/// and convenience methods for use within SecretFs and locket Path handling.
pub trait PathExt {
    /// Cleans the path by removing redundant components like `\\`, `.`, and `..`
    fn clean(&self) -> PathBuf;
    /// Converts the path to an absolute path based on the current working directory
    /// This method does not touch the disk so it will not ensure the file exists,
    /// nor will it resolve symlinks. It will also clean the path.
    /// In the event that the absolute path cannot be determined, it will
    /// return the cleaned version of the original path.
    fn absolute(&self) -> PathBuf;
    /// Small wrapper around canonicalize that returns SecretError
    /// instead of std::io::Error.
    /// This will resolve symlinks and require that the path exists.
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

/// Mapping of source path to destination path for secret files
#[derive(Debug, Clone)]
pub struct PathMapping {
    src: PathBuf,
    dst: PathBuf,
}

impl PathMapping {
    pub fn new(src: impl AsRef<Path>, dst: impl AsRef<Path>) -> Self {
        Self {
            src: src.as_ref().absolute(),
            dst: dst.as_ref().absolute(),
        }
    }
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
