// src/secrets/references.rs

#[cfg(any(feature = "op", feature = "connect"))]
use percent_encoding::percent_decode_str;
use std::str::FromStr;
use thiserror::Error;

/// Errors that can occur when parsing a specific Secret Reference string.
#[derive(Debug, Error)]
pub enum ReferenceParseError {
    #[error("unknown or invalid secret format: {0}")]
    UnknownFormat(String),

    #[cfg(any(feature = "op", feature = "connect"))]
    #[error(transparent)]
    Op(#[from] OpParseError),

    #[cfg(feature = "bws")]
    #[error("invalid BWS UUID: {0}")]
    Bws(#[from] uuid::Error),
}

/// A parsed reference to a secret.
///
/// This enum represents a valid pointer to a secret. It guarantees that the
/// syntax matches the requirements of the specific provider.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SecretReference {
    #[cfg(any(feature = "op", feature = "connect"))]
    /// A 1Password reference (used by both CLI and Connect providers)
    OnePassword(OpReference),

    #[cfg(feature = "bws")]
    /// A Bitwarden Secrets Manager reference (UUID)
    Bws(BwsReference),

    #[cfg(any(test, doctest, feature = "testing"))]
    /// A mock reference for testing purposes
    Mock(String),
}

// TODO: consider going to derive_more to reduce boilerplate,
// which could help with other impls too.
impl<'a> TryFrom<&'a SecretReference> for &'a OpReference {
    type Error = ();

    fn try_from(value: &'a SecretReference) -> Result<Self, Self::Error> {
        if let SecretReference::OnePassword(op) = value {
            Ok(op)
        } else {
            Err(())
        }
    }
}

impl<'a> TryFrom<&'a SecretReference> for &'a BwsReference {
    type Error = ();

    fn try_from(value: &'a SecretReference) -> Result<Self, Self::Error> {
        if let SecretReference::Bws(bws) = value {
            Ok(bws)
        } else {
            Err(())
        }
    }
}

/// Defines how to identify and parse valid secret references from string literals.
pub trait ReferenceParser: Send + Sync {
    fn parse(&self, raw: &str) -> Option<SecretReference>;
}

impl std::fmt::Display for SecretReference {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            #[cfg(any(feature = "op", feature = "connect"))]
            Self::OnePassword(reference) => write!(f, "{}", reference),

            #[cfg(feature = "bws")]
            Self::Bws(uuid) => write!(f, "{}", uuid),

            #[cfg(any(test, doctest, feature = "testing"))]
            Self::Mock(inner) => write!(f, "{}", inner),
        }
    }
}

impl FromStr for SecretReference {
    type Err = ReferenceParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // Check 1Password
        #[cfg(any(feature = "op", feature = "connect"))]
        if s.starts_with("op://") {
            let op_ref = OpReference::from_str(s)?;
            return Ok(Self::OnePassword(op_ref));
        }

        // Check BWS
        #[cfg(feature = "bws")]
        if let Ok(bws_ref) = BwsReference::from_str(s) {
            return Ok(Self::Bws(bws_ref));
        }

        // Fallback
        Err(ReferenceParseError::UnknownFormat(s.to_string()))
    }
}

#[cfg(any(feature = "op", feature = "connect"))]
#[derive(Debug, Error)]
pub enum OpParseError {
    #[error("reference must start with 'op://'")]
    InvalidScheme,

    #[error("invalid URL structure: {0}")]
    UrlParse(#[from] url::ParseError),

    #[error("missing vault name")]
    MissingVault,

    #[error("invalid path segments: expected 2 (item/field) or 3 (item/section/field), got {0}")]
    InvalidSegments(usize),

    #[error("vault, item, or field cannot be empty")]
    EmptyComponent,

    #[error("utf8 decode error: {0}")]
    Utf8(#[from] std::str::Utf8Error),
}

#[cfg(feature = "bws")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BwsReference(uuid::Uuid);

#[cfg(feature = "bws")]
impl std::fmt::Display for BwsReference {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[cfg(feature = "bws")]
impl FromStr for BwsReference {
    type Err = ReferenceParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let uuid = uuid::Uuid::parse_str(s)?;
        Ok(BwsReference(uuid))
    }
}

#[cfg(feature = "bws")]
impl From<uuid::Uuid> for BwsReference {
    fn from(u: uuid::Uuid) -> Self {
        BwsReference(u)
    }
}

#[cfg(feature = "bws")]
impl From<BwsReference> for uuid::Uuid {
    fn from(bws: BwsReference) -> Self {
        bws.0
    }
}

#[cfg(any(feature = "op", feature = "connect"))]
/// Represents a syntactically valid 1Password secret reference.
/// Syntax: `op://<vault>/<item>/[<section>/]<field>[?options]`
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct OpReference {
    /// The raw original string
    raw: String,

    // Parsed components.
    pub vault: String,
    pub item: String,
    pub section: Option<String>,
    pub field: String,
}

#[cfg(any(feature = "op", feature = "connect"))]
impl OpReference {
    pub fn as_str(&self) -> &str {
        &self.raw
    }
}

#[cfg(any(feature = "op", feature = "connect"))]
impl std::fmt::Display for OpReference {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.raw)
    }
}

#[cfg(any(feature = "op", feature = "connect"))]
impl FromStr for OpReference {
    type Err = OpParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if !s.starts_with("op://") {
            return Err(OpParseError::InvalidScheme);
        }

        let url = url::Url::parse(s)?;

        let host_str = url.host_str().ok_or(OpParseError::MissingVault)?;
        let vault = percent_decode_str(host_str)
            .decode_utf8()
            .map_err(OpParseError::Utf8)?
            .to_string();

        let raw_segments = url
            .path_segments()
            .ok_or(OpParseError::InvalidSegments(0))?;

        let mut segments = Vec::new();
        for segment in raw_segments {
            let decoded = percent_decode_str(segment)
                .decode_utf8()
                .map_err(OpParseError::Utf8)?
                .to_string();
            segments.push(decoded);
        }

        let (item, section, field) = match segments.len() {
            2 => (segments[0].clone(), None, segments[1].clone()),
            3 => (
                segments[0].clone(),
                Some(segments[1].clone()),
                segments[2].clone(),
            ),
            _ => return Err(OpParseError::InvalidSegments(segments.len())),
        };

        if vault.is_empty() || item.is_empty() || field.is_empty() {
            return Err(OpParseError::EmptyComponent);
        }

        Ok(Self {
            raw: s.to_string(),
            vault,
            item: item.to_string(),
            section: section.map(|s| s.to_string()),
            field: field.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(any(feature = "op", feature = "connect"))]
    fn test_parse_op_2_segment() {
        let raw = "op://vault/item/field";
        let r = OpReference::from_str(raw).unwrap();
        assert_eq!(r.vault, "vault");
        assert_eq!(r.item, "item");
        assert_eq!(r.section, None);
        assert_eq!(r.field, "field");
    }

    #[test]
    #[cfg(any(feature = "op", feature = "connect"))]
    fn test_parse_op_3_segment() {
        let raw = "op://vault/item/section/field";
        let r = OpReference::from_str(raw).unwrap();
        assert_eq!(r.vault, "vault");
        assert_eq!(r.item, "item");
        assert_eq!(r.section, Some("section".into()));
        assert_eq!(r.field, "field");
    }

    #[test]
    #[cfg(any(feature = "op", feature = "connect"))]
    fn test_parse_op_spaces() {
        // url crate handles percent encoding
        let raw = "op://My%20Vault/My%20Item/field";
        let r = OpReference::from_str(raw).unwrap();
        assert_eq!(r.vault, "My Vault");
        assert_eq!(r.item, "My Item");
        assert_eq!(r.field, "field");
    }

    #[test]
    #[cfg(feature = "bws")]
    fn test_parse_bws() {
        let raw = "3832b656-a93b-45ad-bdfa-b267016802c3";
        let r = SecretReference::from_str(raw).unwrap();
        match r {
            SecretReference::Bws(u) => assert_eq!(u.to_string(), raw),
            _ => panic!("wrong type"),
        }
    }

    #[test]
    fn test_parse_invalid() {
        assert!(SecretReference::from_str("not-a-secret").is_err());
    }
}
