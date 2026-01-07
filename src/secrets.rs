//! Core primitives for secret management and definition.
//!
//! This module defines the `Secret` type, which abstracts over the source of a secret
//! (i.e literal string, file path) and its identification (named vs. anonymous).
//!
//! It also handles the low-level "reading" mechanics via `SecretSource` and `SourceReader`,
//! ensuring that file reads are memory-limited.

use crate::path::CanonicalPath;
use crate::provider::ProviderError;
use serde::Deserialize;
use std::borrow::Cow;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use thiserror::Error;

pub mod config;
mod file;
mod manager;
mod registry;
pub use crate::secrets::config::{InjectFailurePolicy, SecretManagerArgs, SecretManagerConfig};
pub use crate::secrets::manager::SecretFileManager;

#[derive(Debug, Error)]
pub enum SecretError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("provider error: {0}")]
    Provider(#[from] ProviderError),

    #[error("blocking task failed: {0}")]
    Task(#[from] tokio::task::JoinError),

    #[error("input file too large: {size} bytes (limit: {limit} bytes): {path:?}")]
    SourceTooLarge {
        path: PathBuf,
        size: u64,
        limit: u64,
    },

    #[error("collision detected: '{dst:?}' is targeted by multiple sources")]
    Collision {
        first: String,
        second: String,
        dst: PathBuf,
    },

    #[error("structure conflict: {blocker:?} blocks {blocked:?}")]
    StructureConflict { blocker: String, blocked: String },

    #[error("source path missing: {0:?}")]
    SourceMissing(PathBuf),

    #[error("destination {dst:?} is inside source {src:?}")]
    Loop { src: PathBuf, dst: PathBuf },

    #[error("source {src:?} is inside destination {dst:?}")]
    Destructive { src: PathBuf, dst: PathBuf },

    #[error("dst has no parent: {0}")]
    NoParent(PathBuf),

    #[error("parse error: {0}")]
    Parse(String),

    #[error("file write error: {0}")]
    Write(#[from] crate::write::WriterError),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Ord, PartialOrd)]
pub struct SecretKey(String);

impl TryFrom<String> for SecretKey {
    type Error = SecretError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        let trimmed = value.trim();

        if trimmed.is_empty() {
            return Err(SecretError::Parse("Secret key cannot be empty".to_string()));
        }

        if trimmed.contains('=') {
            return Err(SecretError::Parse(
                "Secret key cannot contain '=' character".to_string(),
            ));
        }

        if trimmed.contains('\0') {
            return Err(SecretError::Parse(
                "Secret key cannot contain null bytes".to_string(),
            ));
        }

        // Store the trimmed version
        Ok(SecretKey(trimmed.to_string()))
    }
}

impl AsRef<str> for SecretKey {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl From<SecretKey> for String {
    fn from(val: SecretKey) -> Self {
        val.0
    }
}

impl From<&SecretKey> for String {
    fn from(val: &SecretKey) -> Self {
        val.0.clone()
    }
}

/// The primitive definition of a secret, which is ultimately responsible for holding
/// secret reference data.
///
/// It also holds additional context which indicates whether the secret is named (i.e.
/// has an explicit key) or anonymous (no key, just a source).
///
/// The consumer of the secret is responsible for interpreting the use context.
/// For example, in the SecretFileManager, anonymous secrets are treated the same as Named
/// file backed secrets, except the key is derived from the file name.
/// However in the EnvManager, anonymous secrets are treated as .env files.
#[derive(Debug, Clone, Deserialize)]
#[serde(try_from = "String")]
pub enum Secret {
    /// An anonymous secret source (e.g., a file path or string literal)
    ///
    /// The consumer decides how to handle the secret the context
    ///
    /// Input format: usually `"path/to/file"` or `"@path/to/file"`
    Anonymous(SecretSource),

    /// A named secret with an explicit key.
    ///
    /// Input format: `"KEY=VALUE"` or `"KEY=@path/to/file"`
    Named {
        key: SecretKey,
        source: SecretSource,
    },
}

impl Secret {
    /// Creates a Named secret from a Key/Value pair string.
    ///
    /// If `val` starts with `@`, it is treated as a file path.
    fn from_kv(key: String, val: String) -> Result<Self, SecretError> {
        // If val starts with @, it's a file source. Otherwise, literal.
        let source = if let Some(path) = val.strip_prefix('@') {
            SecretSource::file(path)?
        } else {
            SecretSource::literal(&key, val)
        };

        Ok(Self::Named {
            key: key.try_into()?,
            source,
        })
    }

    /// Creates an Anonymous secret from a file path.
    pub fn from_file(path: impl AsRef<Path>) -> Result<Self, SecretError> {
        let source = SecretSource::file(path)?;
        Ok(Self::Anonymous(source))
    }

    /// Helper to batch convert a Map into Named secrets.
    pub fn try_from_map(map: HashMap<String, String>) -> Result<Vec<Self>, SecretError> {
        map.into_iter().map(|(k, v)| Self::from_kv(k, v)).collect()
    }

    /// Access the inner source definition.
    pub fn source(&self) -> &SecretSource {
        match self {
            Secret::Anonymous(s) => s,
            Secret::Named { source, .. } => source,
        }
    }
}

impl TryFrom<String> for Secret {
    type Error = SecretError;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        s.parse()
    }
}

impl FromStr for Secret {
    type Err = SecretError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // key=value form means Named secret
        if let Some((key, val)) = s.split_once('=') {
            let source = if let Some(path) = val.strip_prefix('@') {
                SecretSource::file(path)?
            } else {
                SecretSource::literal(key, val)
            };

            return Ok(Self::Named {
                key: key.to_string().try_into()?,
                source,
            });
        }

        // No `=` means Anonymous secret.
        // This does mean file paths with `=` may not be parsed correctly.
        // Strip explicit file indicator '@' if present, otherwise treat as path.
        let path = s.strip_prefix('@').unwrap_or(s);
        Ok(Self::Anonymous(SecretSource::file(path)?))
    }
}

/// The origin of the secret template content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SecretSource {
    /// Template loaded from a file path on disk.
    File(CanonicalPath),
    /// Template provided as a raw string literal.
    Literal {
        label: Option<String>,
        template: String,
    },
}

impl SecretSource {
    /// Create a source from a file path.
    ///
    /// # Errors
    /// Returns error if the path cannot be canonicalized (does not exist).
    pub fn file(path: impl AsRef<Path>) -> Result<Self, SecretError> {
        Ok(Self::File(CanonicalPath::try_new(path)?))
    }

    /// Create a source from a string literal.
    pub fn literal(label: impl Into<String>, text: impl Into<String>) -> Self {
        Self::Literal {
            label: Some(label.into()),
            template: text.into(),
        }
    }

    /// Returns a reader to fetch the content.
    pub fn read(&self) -> SourceReader<'_> {
        SourceReader {
            source: self,
            max_size: MemSize::MAX,
        }
    }

    /// Returns a label describing the source.
    pub fn label(&self) -> Cow<'_, str> {
        match self {
            Self::File(p) => p.to_string_lossy(),
            Self::Literal { label, .. } => {
                Cow::Borrowed(label.as_deref().unwrap_or("inline-value"))
            }
        }
    }

    /// If the source is a file, returns its path.
    pub fn path(&self) -> Option<&CanonicalPath> {
        match self {
            SecretSource::File(p) => Some(p),
            SecretSource::Literal { .. } => None,
        }
    }
}

/// Reader for fetching secret source content.
pub struct SourceReader<'a> {
    source: &'a SecretSource,
    max_size: MemSize,
}

impl<'a> SourceReader<'a> {
    /// Apply a maximum size limit (in bytes) to the read operation.
    /// Because the reader buffers the entire source into memory, this may be necessary for consumers
    /// to avoid excessive memory usage.
    pub fn limit(mut self, size: MemSize) -> Self {
        self.max_size = size;
        self
    }

    /// Fetches the content.
    ///
    /// # Errors
    /// Returns `SecretError::SourceTooLarge` if the file exceeds the configured limit.
    pub fn fetch(self) -> Result<Option<Cow<'a, str>>, SecretError> {
        match self.source {
            SecretSource::File(path) => match std::fs::metadata(path) {
                Ok(meta) => {
                    if meta.len() > self.max_size.bytes {
                        return Err(SecretError::SourceTooLarge {
                            path: path.to_path_buf(),
                            size: meta.len(),
                            limit: self.max_size.bytes,
                        });
                    }
                    let c = std::fs::read_to_string(path).map_err(SecretError::Io)?;
                    Ok(Some(Cow::Owned(c)))
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
                Err(e) => Err(SecretError::Io(e)),
            },
            SecretSource::Literal { template, .. } => Ok(Some(Cow::Borrowed(template))),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
#[serde(try_from = "String")]
pub struct MemSize {
    pub bytes: u64,
}

impl TryFrom<String> for MemSize {
    type Error = String;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        s.parse()
    }
}

impl Default for MemSize {
    fn default() -> Self {
        Self {
            bytes: 10 * 1024 * 1024,
        }
    }
}

impl MemSize {
    pub const MAX: Self = Self { bytes: u64::MAX };
    pub fn new(bytes: u64) -> Self {
        Self { bytes }
    }
    pub fn from_mb(mb: u64) -> Self {
        Self {
            bytes: mb.saturating_mul(1024 * 1024),
        }
    }
    pub fn from_kb(kb: u64) -> Self {
        Self {
            bytes: kb.saturating_mul(1024),
        }
    }
    pub fn from_gb(gb: u64) -> Self {
        Self {
            bytes: gb.saturating_mul(1024 * 1024 * 1024),
        }
    }
}

impl std::str::FromStr for MemSize {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim();
        let digit_end = s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
        let (num_str, suffix) = s.split_at(digit_end);

        if num_str.is_empty() {
            return Err("No number provided".to_string());
        }

        let num: u64 = num_str
            .parse()
            .map_err(|e| format!("Invalid number: {}", e))?;

        match suffix.trim().to_ascii_lowercase().as_str() {
            "" | "b" | "byte" | "bytes" => Ok(MemSize::new(num)),
            "k" | "kb" | "kib" => Ok(MemSize::from_kb(num)),
            "m" | "mb" | "mib" => Ok(MemSize::from_mb(num)),
            "g" | "gb" | "gib" => Ok(MemSize::from_gb(num)),
            _ => Err(format!(
                "Unknown size suffix: '{}'. Supported: k, m, g",
                suffix
            )),
        }
    }
}

impl std::fmt::Display for MemSize {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.bytes >= 1024 * 1024 * 1024 && self.bytes.is_multiple_of(1024 * 1024 * 1024) {
            write!(f, "{}G", self.bytes / (1024 * 1024 * 1024))
        } else if self.bytes >= 1024 * 1024 && self.bytes.is_multiple_of(1024 * 1024) {
            write!(f, "{}M", self.bytes / (1024 * 1024))
        } else {
            write!(f, "{}B", self.bytes)
        }
    }
}
