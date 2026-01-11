use super::Overlay;
use serde::{Deserialize, Serialize};
use std::str::FromStr;

/// A wrapper around `Vec<T>` that implements `Overlay` by appending
/// rather than replacing.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct MergeVec<T>(pub Vec<T>);

impl<T> Default for MergeVec<T> {
    fn default() -> Self {
        Self(Vec::new())
    }
}

impl<T> MergeVec<T> {
    pub fn new() -> Self {
        Self(Vec::new())
    }
}

impl<T> From<Vec<T>> for MergeVec<T> {
    fn from(v: Vec<T>) -> Self {
        Self(v)
    }
}

impl<T> From<MergeVec<T>> for Vec<T> {
    fn from(v: MergeVec<T>) -> Self {
        v.0
    }
}

impl<T> Overlay for MergeVec<T> {
    fn overlay(mut self, over: Self) -> Self {
        self.0.extend(over.0);
        self
    }
}

impl<T> FromStr for MergeVec<T>
where
    T: FromStr,
{
    type Err = T::Err;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.is_empty() {
            return Ok(Self(Vec::new()));
        }
        T::from_str(s).map(|item| Self(vec![item]))
    }
}

impl<T> std::fmt::Display for MergeVec<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "")
    }
}
