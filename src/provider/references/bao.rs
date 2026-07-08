//! Defines the OpenBao / Vault secret reference type and its parsing logic.
use super::{Extract, ReferenceSyntax, SecretReference};
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, percent_decode_str, utf8_percent_encode};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;
use thiserror::Error;

/// Escapes everything outside the RFC 3986 unreserved set so that every
/// component survives a display/parse round trip.
const COMPONENT: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'~');

#[derive(Debug, Error)]
pub enum BaoParseError {
    #[error("reference must start with 'bao://'")]
    InvalidScheme,

    #[error("invalid URL structure: {0}")]
    UrlParse(#[from] url::ParseError),

    #[error("missing mount name")]
    MissingMount,

    #[error("invalid path segments: expected at least 2 (path/field), got {0}")]
    InvalidSegments(usize),

    #[error("validation error: {0}")]
    Validation(#[from] ValidationError),

    #[error("utf8 decode error: {0}")]
    Utf8(#[from] std::str::Utf8Error),
}

#[derive(Debug, Error)]
pub enum ValidationError {
    #[error("invalid mount '{0}': cannot be empty or contain '/'")]
    Mount(String),

    #[error("invalid path '{0}': segments cannot be empty")]
    Path(String),

    #[error("field cannot be empty")]
    Field,
}

/// The path where a KV v2 secrets engine is mounted (e.g. `secret`).
///
/// Restricted to a single path segment: the mount occupies the host position
/// of the `bao://` syntax, so mounts nested under a `/` cannot be expressed.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct BaoMount(String);

impl BaoMount {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for BaoMount {
    type Error = ValidationError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        if value.is_empty() || value.contains('/') {
            return Err(ValidationError::Mount(value));
        }
        Ok(Self(value))
    }
}

impl FromStr for BaoMount {
    type Err = ValidationError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::try_from(s.to_string())
    }
}

impl From<BaoMount> for String {
    fn from(mount: BaoMount) -> Self {
        mount.0
    }
}

impl AsRef<str> for BaoMount {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for BaoMount {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A secret's path within a KV v2 engine: one or more non-empty segments.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct BaoPath(Vec<String>);

impl BaoPath {
    pub fn new(segments: Vec<String>) -> Result<Self, ValidationError> {
        if segments.is_empty() || segments.iter().any(String::is_empty) {
            return Err(ValidationError::Path(segments.join("/")));
        }
        Ok(Self(segments))
    }

    pub fn segments(&self) -> impl Iterator<Item = &str> {
        self.0.iter().map(String::as_str)
    }
}

impl fmt::Display for BaoPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut segments = self.segments();
        if let Some(first) = segments.next() {
            f.write_str(first)?;
        }
        for segment in segments {
            write!(f, "/{}", segment)?;
        }
        Ok(())
    }
}

/// A key within a secret's data map.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct BaoField(String);

impl BaoField {
    pub fn new(s: impl Into<String>) -> Result<Self, ValidationError> {
        let s = s.into();
        if s.is_empty() {
            return Err(ValidationError::Field);
        }
        Ok(Self(s))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for BaoField {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// The (mount, path) pair addressing a single KV v2 secret.
///
/// Every referenced field of that secret lives in the same data map, so
/// references are grouped and fetched by location.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct BaoSecretLocation {
    pub mount: BaoMount,
    pub path: BaoPath,
}

impl fmt::Display for BaoSecretLocation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.mount, self.path)
    }
}

/// Represents a syntactically valid OpenBao / Vault secret reference.
/// Syntax: `bao://<mount>/<path>/<field>`
///
/// * `mount` is the path where the KV v2 secrets engine is mounted (e.g. `secret`)
/// * `path` is the secret's path within that engine, may contain nested segments (e.g. `app/prod`)
/// * `field` is the specific key within the secret's data map
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct BaoReference {
    pub location: BaoSecretLocation,
    pub field: BaoField,
}

impl FromStr for BaoReference {
    type Err = BaoParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if !s.starts_with("bao://") {
            return Err(BaoParseError::InvalidScheme);
        }

        let url = url::Url::parse(s)?;

        let host = url.host_str().ok_or(BaoParseError::MissingMount)?;
        let mount = BaoMount::try_from(percent_decode_str(host).decode_utf8()?.into_owned())?;

        let raw_segments = url
            .path_segments()
            .ok_or(BaoParseError::InvalidSegments(0))?;

        let mut segments = Vec::new();
        for segment in raw_segments {
            segments.push(percent_decode_str(segment).decode_utf8()?.into_owned());
        }

        if segments.len() < 2 {
            return Err(BaoParseError::InvalidSegments(segments.len()));
        }

        let field = segments.pop().ok_or(BaoParseError::InvalidSegments(0))?;
        let field = BaoField::new(field)?;
        let path = BaoPath::new(segments)?;

        Ok(Self {
            location: BaoSecretLocation { mount, path },
            field,
        })
    }
}

impl From<BaoReference> for SecretReference {
    fn from(r: BaoReference) -> Self {
        Self::Bao(r)
    }
}

impl ReferenceSyntax for BaoReference {
    fn try_parse(raw: &str) -> Option<Self> {
        Self::from_str(raw)
            .inspect_err(|e| {
                if !matches!(e, BaoParseError::InvalidScheme) {
                    tracing::warn!("Invalid OpenBao reference '{}': {}", raw, e);
                }
            })
            .ok()
    }
}

impl Extract for BaoReference {
    fn extract(r: &SecretReference) -> Option<&Self> {
        #[allow(unreachable_patterns)]
        match r {
            SecretReference::Bao(inner) => Some(inner),
            _ => None,
        }
    }
}

impl fmt::Display for BaoReference {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "bao://{}",
            utf8_percent_encode(self.location.mount.as_str(), COMPONENT)
        )?;
        for segment in self.location.path.segments() {
            write!(f, "/{}", utf8_percent_encode(segment, COMPONENT))?;
        }
        write!(
            f,
            "/{}",
            utf8_percent_encode(self.field.as_str(), COMPONENT)
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_bao_simple() {
        let raw = "bao://secret/app/password";
        let r = BaoReference::from_str(raw).unwrap();
        assert_eq!(r.location.mount.as_str(), "secret");
        assert_eq!(r.location.path.to_string(), "app");
        assert_eq!(r.field.as_str(), "password");
    }

    #[test]
    fn test_parse_bao_nested_path() {
        let raw = "bao://secret/app/prod/db/password";
        let r = BaoReference::from_str(raw).unwrap();
        assert_eq!(r.location.mount.as_str(), "secret");
        assert_eq!(r.location.path.to_string(), "app/prod/db");
        assert_eq!(r.field.as_str(), "password");
    }

    #[test]
    fn test_parse_bao_too_few_segments() {
        let raw = "bao://secret/password";
        let err = BaoReference::from_str(raw);
        assert!(matches!(err, Err(BaoParseError::InvalidSegments(1))));
    }

    #[test]
    fn test_parse_bao_spaces() {
        let raw = "bao://secret/My%20App/password";
        let r = BaoReference::from_str(raw).unwrap();
        assert_eq!(r.location.mount.as_str(), "secret");
        assert_eq!(r.location.path.to_string(), "My App");
        assert_eq!(r.field.as_str(), "password");
    }

    #[test]
    fn test_parse_bao_invalid_scheme() {
        let raw = "http://secret/app/password";
        let err = BaoReference::from_str(raw);
        assert!(matches!(err, Err(BaoParseError::InvalidScheme)));
    }

    #[test]
    fn test_parse_bao_empty_path_segment() {
        let raw = "bao://secret//password";
        let err = BaoReference::from_str(raw);
        assert!(matches!(
            err,
            Err(BaoParseError::Validation(ValidationError::Path(_)))
        ));
    }

    #[test]
    fn test_parse_bao_empty_field() {
        let raw = "bao://secret/app/";
        let err = BaoReference::from_str(raw);
        assert!(matches!(
            err,
            Err(BaoParseError::Validation(ValidationError::Field))
        ));
    }

    #[test]
    fn test_parse_bao_missing_mount() {
        let raw = "bao:///app/password";
        let err = BaoReference::from_str(raw);
        assert!(matches!(err, Err(BaoParseError::MissingMount)));
    }

    #[test]
    fn test_mount_rejects_slash() {
        assert!(BaoMount::from_str("team/kv").is_err());
        assert!(BaoMount::from_str("").is_err());
        assert!(BaoMount::from_str("secret").is_ok());
    }

    #[test]
    fn test_display_round_trip() {
        let raw = "bao://secret/app/prod/db/password";
        let r = BaoReference::from_str(raw).unwrap();
        assert_eq!(r.to_string(), raw);
        assert_eq!(BaoReference::from_str(&r.to_string()).unwrap(), r);
    }

    #[test]
    fn test_display_round_trip_encoded() {
        let raw = "bao://My%20Mount/My%20App/pass%2Fword";
        let r = BaoReference::from_str(raw).unwrap();
        assert_eq!(r.location.mount.as_str(), "My Mount");
        assert_eq!(r.location.path.to_string(), "My App");
        assert_eq!(r.field.as_str(), "pass/word");
        assert_eq!(r.to_string(), raw);
        assert_eq!(BaoReference::from_str(&r.to_string()).unwrap(), r);
    }

    #[test]
    fn test_display_canonicalizes() {
        // Gratuitous escapes decode on parse, so semantically equal
        // references display identically and compare equal.
        let canonical = BaoReference::from_str("bao://secret/app/password").unwrap();
        let escaped = BaoReference::from_str("bao://secret/app/%70assword").unwrap();
        assert_eq!(escaped, canonical);
        assert_eq!(escaped.to_string(), "bao://secret/app/password");
    }
}
