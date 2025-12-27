use super::{MemSize, Secret, SecretError, SecretSource};
use crate::path::{AbsolutePath, CanonicalPath};
use std::borrow::Cow;
use std::path::Path;

/// Representation of a secret file, which contains secret references
/// and is intended to be materialized to a specific destination path.
#[derive(Debug, Clone)]
pub struct SecretFile {
    source: SecretSource,
    dest: AbsolutePath,
    max_size: MemSize,
}

impl SecretFile {
    pub fn from_file(
        src: CanonicalPath,
        dest: AbsolutePath,
        max_size: MemSize,
    ) -> Result<Self, SecretError> {
        Ok(Self {
            source: SecretSource::File(src),
            dest,
            max_size,
        })
    }
    pub fn from_template(label: String, template: String, root: &AbsolutePath) -> Self {
        let safe_name = sanitize_filename::sanitize(&label);
        let dest = root.join(safe_name);
        Self {
            source: SecretSource::literal(label, template),
            dest: AbsolutePath::new(dest),
            max_size: MemSize::MAX,
        }
    }
    pub fn from_secret(
        secret: Secret,
        root: &AbsolutePath,
        max_size: MemSize,
    ) -> Result<Self, SecretError> {
        let (key, source) = match secret {
            Secret::Named { key, source } => (key, source),
            Secret::Anonymous(source) => {
                let path = source.path().ok_or_else(|| {
                    SecretError::Parse(
                        "Cannot derive SecretFile from anonymous literal secret".to_string(),
                    )
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
        let dest = root.join(safe_name);

        Ok(Self {
            source,
            dest: AbsolutePath::new(dest),
            max_size,
        })
    }

    pub fn dest(&self) -> &AbsolutePath {
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
                let path = self
                    .source
                    .path()
                    .map(|p| p.as_ref())
                    .unwrap_or_else(|| Path::new("<unknown>"));
                SecretError::SourceMissing(path.to_path_buf())
            })
    }
}
