//! Defines the Bitwarden Secrets (BWS) reference type and its parsing logic.
use super::{ReferenceParseError, ReferenceSyntax, SecretReference};
use std::str::FromStr;
use uuid::Uuid;

/// Represents a syntactically valid Bitwarden Secrets Manager secret reference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BwsReference(Uuid);

impl From<BwsReference> for SecretReference {
    fn from(r: BwsReference) -> Self {
        Self::Bws(r)
    }
}

impl ReferenceSyntax for BwsReference {
    fn try_parse(raw: &str) -> Option<Self> {
        Self::from_str(raw).ok()
    }
}

impl FromStr for BwsReference {
    type Err = ReferenceParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let uuid = Uuid::parse_str(s)?;
        Ok(BwsReference(uuid))
    }
}

impl std::fmt::Display for BwsReference {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<Uuid> for BwsReference {
    fn from(u: Uuid) -> Self {
        BwsReference(u)
    }
}

impl From<BwsReference> for Uuid {
    fn from(bws: BwsReference) -> Self {
        bws.0
    }
}

impl<'a> TryFrom<&'a SecretReference> for &'a BwsReference {
    type Error = ();

    #[allow(irrefutable_let_patterns)]
    fn try_from(value: &'a SecretReference) -> Result<Self, Self::Error> {
        if let SecretReference::Bws(bws) = value {
            Ok(bws)
        } else {
            Err(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_bws() {
        let raw = "3832b656-a93b-45ad-bdfa-b267016802c3";
        let r = SecretReference::from_str(raw).unwrap();
        match r {
            SecretReference::Bws(u) => assert_eq!(u.to_string(), raw),
            _ => panic!("wrong type"),
        }
    }
}
