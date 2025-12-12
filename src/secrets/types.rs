use crate::provider::ProviderError;
use crate::secrets::path::PathExt;
use clap::ValueEnum;
use std::borrow::Cow;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use thiserror::Error;

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
}

#[derive(Debug, Clone)]
pub enum Secret {
    /// An anonymous secret source.
    /// The consumer decides how to handle it.
    /// Input: "path/to/file" or "@path/to/file"
    Anonymous(SecretSource),

    /// A named secret with an explicit key.
    /// Input: "KEY=VALUE" or "KEY=@path/to/file"
    Named { key: String, source: SecretSource },
}

impl Secret {
    /// Creates a Named secret from a Key/Value pair.
    fn from_kv(key: String, val: String) -> Result<Self, SecretError> {
        // If val starts with @, it's a file source. Otherwise, literal.
        let source = if let Some(path) = val.strip_prefix('@') {
            SecretSource::file(path)?
        } else {
            SecretSource::literal(&key, val)
        };

        Ok(Self::Named { key, source })
    }

    pub fn from_file(path: impl AsRef<Path>) -> Result<Self, SecretError> {
        let source = SecretSource::file(path)?;
        Ok(Self::Anonymous(source))
    }

    /// Helper to batch convert a HashMap into Named secrets
    pub fn try_from_map(map: HashMap<String, String>) -> Result<Vec<Self>, SecretError> {
        map.into_iter().map(|(k, v)| Self::from_kv(k, v)).collect()
    }

    /// Utility to access the inner source regardless of variant.
    pub fn source(&self) -> &SecretSource {
        match self {
            Secret::Anonymous(s) => s,
            Secret::Named { source, .. } => source,
        }
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
                key: key.to_string(),
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

#[derive(Copy, Clone, Debug, ValueEnum, Default)]
pub enum InjectFailurePolicy {
    /// Injection failures are treated as errors and will abort the process
    Error,
    /// On injection failure, copy the unmodified secret to destination
    #[default]
    CopyUnmodified,
    /// On injection failure, just log a warning and proceed with the secret ignored
    Ignore,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SecretSource {
    /// Template loaded from a file path
    File(PathBuf),
    /// Template loaded from a string literal
    Literal {
        label: Option<String>,
        template: String,
    },
}

impl SecretSource {
    pub fn file(path: impl AsRef<Path>) -> Result<Self, SecretError> {
        let canon = path.as_ref().canon()?;
        Ok(Self::File(canon))
    }
    pub fn literal(label: impl Into<String>, text: impl Into<String>) -> Self {
        Self::Literal {
            label: Some(label.into()),
            template: text.into(),
        }
    }
    pub fn read(&self) -> SourceReader<'_> {
        SourceReader {
            source: self,
            max_size: MemSize::MAX,
        }
    }
    pub fn label(&self) -> Cow<'_, str> {
        match self {
            Self::File(p) => p.to_string_lossy(),
            Self::Literal { label, .. } => {
                Cow::Borrowed(label.as_deref().unwrap_or("inline-value"))
            }
        }
    }
    pub fn path(&self) -> Option<&Path> {
        match self {
            SecretSource::File(p) => Some(p),
            SecretSource::Literal { .. } => None,
        }
    }
}

impl From<PathBuf> for SecretSource {
    fn from(p: PathBuf) -> Self {
        Self::File(p)
    }
}

pub struct SourceReader<'a> {
    source: &'a SecretSource,
    max_size: MemSize,
}

impl<'a> SourceReader<'a> {
    pub fn limit(mut self, size: MemSize) -> Self {
        self.max_size = size;
        self
    }

    pub fn fetch(self) -> Result<Option<Cow<'a, str>>, SecretError> {
        match self.source {
            SecretSource::File(path) => match std::fs::metadata(path) {
                Ok(meta) => {
                    if meta.len() > self.max_size.bytes {
                        return Err(SecretError::SourceTooLarge {
                            path: path.clone(),
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

#[derive(Debug, Clone)]
pub struct SecretFile {
    source: SecretSource,
    dest: PathBuf,
    max_size: MemSize,
}

impl SecretFile {
    pub fn from_file(
        src: impl AsRef<Path>,
        dest: impl AsRef<Path>,
        max_size: MemSize,
    ) -> Result<Self, SecretError> {
        Ok(Self {
            source: SecretSource::file(src)?,
            dest: dest.as_ref().absolute(),
            max_size,
        })
    }
    pub fn from_template(label: String, template: String, root: &Path) -> Self {
        let safe_name = sanitize_filename::sanitize(&label);
        let dest = root.absolute().join(safe_name);
        Self {
            source: SecretSource::literal(label, template),
            dest,
            max_size: MemSize::MAX,
        }
    }
    pub fn from_secret(
        secret: Secret,
        root: &Path,
        max_size: MemSize,
    ) -> Result<Self, SecretError> {
        let (key, source) = match secret {
            Secret::Named { key, source } => (key, source),
            Secret::Anonymous(source) => {
                let path = source.path().ok_or_else(|| {
                    SecretError::Parse(format!(
                        "Cannot derive SecretFile from anonymous literal secret"
                    ))
                })?;

                let filename = path.file_name().and_then(|s| s.to_str()).ok_or_else(|| {
                    SecretError::Parse(format!(
                        "Could not derive a valid filename from path: {:?}",
                        path
                    ))
                })?;

                (filename.to_string(), source)
            }
        };

        let safe_name = sanitize_filename::sanitize(&key);
        let dest = root.absolute().join(safe_name);

        Ok(Self {
            source,
            dest,
            max_size,
        })
    }

    pub fn dest(&self) -> &Path {
        &self.dest
    }
    pub fn source(&self) -> &SecretSource {
        &self.source
    }

    pub fn content(&self) -> Result<Cow<'_, str>, SecretError> {
        self.source
            .read()
            .limit(self.max_size)
            .fetch()?
            .ok_or_else(|| {
                let path = self.source.path().unwrap_or_else(|| Path::new("<unknown>"));
                SecretError::SourceMissing(path.to_path_buf())
            })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct MemSize {
    pub bytes: u64,
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
