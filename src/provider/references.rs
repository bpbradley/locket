//! Defines secret reference types and parsing logic for each supported provider.
use std::str::FromStr;
use thiserror::Error;

#[cfg(feature = "bws")]
mod bws;
#[cfg(feature = "infisical")]
mod infisical;
#[cfg(any(feature = "op", feature = "connect"))]
mod op;
#[cfg(feature = "bws")]
pub use bws::BwsReference;
#[cfg(feature = "infisical")]
pub use infisical::{
    InfisicalParseError, InfisicalPath, InfisicalProjectId, InfisicalReference,
    InfisicalSecretType, InfisicalSlug,
};
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

    #[cfg(feature = "infisical")]
    #[error(transparent)]
    Infisical(#[from] InfisicalParseError),
}

/// A parsed reference to a secret.
///
/// This enum represents a valid pointer to a secret. It guarantees that the
/// syntax matches the requirements of the specific provider.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SecretReference {
    #[cfg(any(feature = "op", feature = "connect"))]
    /// A 1Password reference
    OnePassword(OpReference),

    #[cfg(feature = "bws")]
    /// A Bitwarden Secrets Manager reference (UUID)
    Bws(BwsReference),

    #[cfg(feature = "infisical")]
    /// An Infisical reference
    Infisical(InfisicalReference),

    #[cfg(any(test, doctest, feature = "testing"))]
    /// A mock reference for testing purposes
    Mock(String),
}

/// Defines the strict syntax rules for a specific reference type.
///
/// This trait is a static factory that attempts to construct a concrete type (Self)
/// from a raw string. It should contain the regex, prefix checking, or validation
/// logic specific to a single provider
pub trait ReferenceSyntax: Sized {
    fn try_parse(raw: &str) -> Option<Self>;
}

/// A runtime interface for providers or configurations that can detect secret references.
///
/// Unlike `ReferenceSyntax`, which is specific to a concrete type, this trait
/// returns the unified `SecretReference`.
pub trait ReferenceParser: Send + Sync {
    fn parse(&self, raw: &str) -> Option<SecretReference>;
}

/// Downcasting trait which defines how to safely extract a concrete reference type from the generic `SecretReference` enum.
pub trait Narrow: Sized {
    fn narrow(r: &SecretReference) -> Option<&Self>;
}

/// Links a provider to its specific reference implementation.
pub trait HasReference {
    type Reference: ReferenceSyntax + Into<SecretReference> + Narrow;
}

/// Blanket implementation for `ReferenceParser` that implements the `HasReference` trait.
impl<T> ReferenceParser for T
where
    T: HasReference + Send + Sync,
{
    fn parse(&self, raw: &str) -> Option<SecretReference> {
        let parsed = T::Reference::try_parse(raw)?;
        Some(parsed.into())
    }
}

impl std::fmt::Display for SecretReference {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            #[cfg(any(feature = "op", feature = "connect"))]
            Self::OnePassword(reference) => write!(f, "{}", reference),

            #[cfg(feature = "bws")]
            Self::Bws(reference) => write!(f, "{}", reference),

            #[cfg(feature = "infisical")]
            Self::Infisical(reference) => write!(f, "{}", reference),

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

        // Check Infisical
        #[cfg(feature = "infisical")]
        if s.starts_with("infisical://") {
            let infisical_ref = InfisicalReference::from_str(s)?;
            return Ok(Self::Infisical(infisical_ref));
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
