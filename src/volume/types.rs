use crate::error::LocketError;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::ops::Deref;
use std::str::FromStr;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String")]
pub struct VolumeName(String);

impl VolumeName {
    pub fn new<S: Into<String>>(name: S) -> Result<Self, LocketError> {
        let s = name.into();
        Self::validate(&s)?;
        Ok(Self(s))
    }

    fn validate(s: &str) -> Result<(), LocketError> {
        if s.is_empty() {
            return Err(LocketError::Validation(
                "Volume name cannot be empty".into(),
            ));
        }
        if s.contains('/') {
            return Err(LocketError::Validation(format!(
                "Volume name cannot contain slashes: '{}'",
                s
            )));
        }
        if s.contains('\0') {
            return Err(LocketError::Validation(
                "Volume name cannot contain null bytes".into(),
            ));
        }
        Ok(())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for VolumeName {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl Deref for VolumeName {
    type Target = str;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl fmt::Display for VolumeName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl TryFrom<String> for VolumeName {
    type Error = LocketError;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        Self::new(s)
    }
}

impl FromStr for VolumeName {
    type Err = LocketError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(s)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String")]
pub struct MountId(String);

impl MountId {
    pub fn new<S: Into<String>>(id: S) -> Result<Self, LocketError> {
        let s = id.into();
        if s.is_empty() {
            return Err(LocketError::Validation("Mount ID cannot be empty".into()));
        }
        Ok(Self(s))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for MountId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl Deref for MountId {
    type Target = str;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl fmt::Display for MountId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl TryFrom<String> for MountId {
    type Error = LocketError;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        Self::new(s)
    }
}

impl FromStr for MountId {
    type Err = LocketError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(s)
    }
}
