use super::SecretReference;
use percent_encoding::percent_decode_str;
use std::str::FromStr;
use thiserror::Error;

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

impl OpReference {
    pub fn as_str(&self) -> &str {
        &self.raw
    }
}

impl std::fmt::Display for OpReference {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.raw)
    }
}

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
    fn test_parse_op_2_segment() {
        let raw = "op://vault/item/field";
        let r = OpReference::from_str(raw).unwrap();
        assert_eq!(r.vault, "vault");
        assert_eq!(r.item, "item");
        assert_eq!(r.section, None);
        assert_eq!(r.field, "field");
    }

    #[test]
    fn test_parse_op_3_segment() {
        let raw = "op://vault/item/section/field";
        let r = OpReference::from_str(raw).unwrap();
        assert_eq!(r.vault, "vault");
        assert_eq!(r.item, "item");
        assert_eq!(r.section, Some("section".into()));
        assert_eq!(r.field, "field");
    }

    #[test]
    fn test_parse_op_spaces() {
        // url crate handles percent encoding
        let raw = "op://My%20Vault/My%20Item/field";
        let r = OpReference::from_str(raw).unwrap();
        assert_eq!(r.vault, "My Vault");
        assert_eq!(r.item, "My Item");
        assert_eq!(r.field, "field");
    }
}
