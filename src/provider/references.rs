// src/secrets/references.rs
use std::str::FromStr;
use thiserror::Error;

#[cfg(feature = "bws")]
mod bws;
#[cfg(any(feature = "op", feature = "connect"))]
mod op;
#[cfg(feature = "bws")]
pub use bws::BwsReference;
#[cfg(any(feature = "op", feature = "connect"))]
pub use op::{OpParseError, OpReference};

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
            Self::Bws(reference) => write!(f, "{}", reference),

            #[cfg(any(test, doctest, feature = "testing"))]
            Self::Mock(reference) => write!(f, "{}", reference),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_invalid() {
        assert!(SecretReference::from_str("not-a-secret").is_err());
    }
}
