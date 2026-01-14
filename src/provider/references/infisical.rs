use super::SecretReference;
use clap::ValueEnum;
use percent_encoding::percent_decode_str;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;
use std::sync::LazyLock;
use thiserror::Error;
use url::Url;
use uuid::Uuid;

// slugs can be lowercase, numbers, or hyphens only
static SLUG_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[a-z0-9-]+$").expect("regex must be valid"));

// keys cannot contain slashes, control characters, or colon
static KEY_INVALID_CHARS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[:/?\x00-\x1f]").expect("regex must be valid"));

// paths must begin with / and contain only alphanumerics and dashes
static PATH_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^/[a-zA-Z0-9_/-]*$").expect("regex must be valid"));

#[derive(Debug, Error)]
pub enum InfisicalParseError {
    #[error("reference must start with 'infisical://'")]
    InvalidScheme,

    #[error("missing secret key in path")]
    MissingKey,

    #[error("invalid URL format: {0}")]
    Url(#[from] url::ParseError),

    #[error("utf8 decode error: {0}")]
    Utf8(#[from] std::str::Utf8Error),

    #[error("validation error: {0}")]
    Validation(#[from] ValidationError),

    #[error("query param error: {0}")]
    QueryParam(#[from] serde_urlencoded::de::Error),
}

#[derive(Debug, Error)]
pub enum ValidationError {
    #[error("invalid slug '{0}': must contain only lowercase letters, numbers, and hyphens")]
    Slug(String),

    #[error("invalid secret key '{0}': cannot contain slashes, colons, or control characters")]
    Key(String),

    #[error("invalid project ID '{0}': must be a valid UUID")]
    ProjectId(String),

    #[error("invalid path '{0}': must start with '/' and contain only alphanumerics and dashes")]
    Path(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct InfisicalReference {
    pub key: InfisicalSecretKey,
    pub options: InfisicalOptions,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct InfisicalOptions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env: Option<InfisicalSlug>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<InfisicalPath>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<InfisicalProjectId>,

    #[serde(default, rename = "type", skip_serializing_if = "Option::is_none")]
    pub secret_type: Option<InfisicalSecretType>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct InfisicalSlug(String);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct InfisicalSecretKey(String);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct InfisicalProjectId(Uuid);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct InfisicalPath(String);

#[derive(Debug, Serialize, Default, Deserialize, Clone, PartialEq, Eq, Hash, ValueEnum, Copy)]
#[serde(rename_all = "lowercase")]
pub enum InfisicalSecretType {
    #[default]
    Shared,
    Personal,
}

impl std::fmt::Display for InfisicalSecretType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.to_possible_value()
            .expect("no values are skipped")
            .get_name()
            .fmt(f)
    }
}

impl<'a> TryFrom<&'a SecretReference> for &'a InfisicalReference {
    type Error = ();

    #[allow(irrefutable_let_patterns)]
    fn try_from(value: &'a SecretReference) -> Result<Self, Self::Error> {
        if let SecretReference::Infisical(inf) = value {
            Ok(inf)
        } else {
            Err(())
        }
    }
}

impl FromStr for InfisicalReference {
    type Err = InfisicalParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if !s.starts_with("infisical://") {
            return Err(InfisicalParseError::InvalidScheme);
        }

        let url = Url::parse(s)?;
        let path = url.path();
        let raw_key = path.strip_prefix('/').unwrap_or(path);

        if raw_key.is_empty() {
            return Err(InfisicalParseError::MissingKey);
        }

        let decoded_key = percent_decode_str(raw_key)
            .decode_utf8()
            .map_err(InfisicalParseError::Utf8)?
            .to_string();

        let key =
            InfisicalSecretKey::parse(decoded_key).map_err(InfisicalParseError::Validation)?;

        let options: InfisicalOptions = serde_urlencoded::from_str(url.query().unwrap_or(""))?;

        Ok(Self { key, options })
    }
}

impl fmt::Display for InfisicalReference {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut url = Url::parse("infisical://").map_err(|_| fmt::Error)?;

        url.set_path(self.key.as_str());

        let query = serde_urlencoded::to_string(&self.options).map_err(|_| fmt::Error)?;
        if !query.is_empty() {
            url.set_query(Some(&query));
        }

        write!(f, "{}", url)
    }
}

impl TryFrom<String> for InfisicalSlug {
    type Error = ValidationError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        if !SLUG_RE.is_match(&value) {
            return Err(ValidationError::Slug(value));
        }
        Ok(Self(value))
    }
}

impl FromStr for InfisicalSlug {
    type Err = ValidationError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::try_from(s.to_string())
    }
}

impl From<InfisicalSlug> for String {
    fn from(slug: InfisicalSlug) -> Self {
        slug.0
    }
}

impl AsRef<str> for InfisicalSlug {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for InfisicalSlug {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl InfisicalSecretKey {
    pub fn parse(s: impl Into<String>) -> Result<Self, ValidationError> {
        let s = s.into();
        if KEY_INVALID_CHARS.is_match(&s) {
            return Err(ValidationError::Key(s));
        }
        Ok(Self(s))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for InfisicalSecretKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl TryFrom<String> for InfisicalProjectId {
    type Error = ValidationError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        let uuid = Uuid::parse_str(&value)
            .map_err(|_| ValidationError::ProjectId(format!("'{}' is not a valid UUID", value)))?;
        Ok(Self(uuid))
    }
}

impl FromStr for InfisicalProjectId {
    type Err = ValidationError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::try_from(s.to_string())
    }
}

impl From<Uuid> for InfisicalProjectId {
    fn from(uuid: Uuid) -> Self {
        Self(uuid)
    }
}

impl From<InfisicalProjectId> for String {
    fn from(pid: InfisicalProjectId) -> Self {
        pid.0.to_string()
    }
}

impl fmt::Display for InfisicalProjectId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl InfisicalPath {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for InfisicalPath {
    type Err = ValidationError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::try_from(s.to_string())
    }
}

impl TryFrom<String> for InfisicalPath {
    type Error = ValidationError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        if !PATH_RE.is_match(&value) {
            return Err(ValidationError::Path(value));
        }
        Ok(Self(value))
    }
}

impl AsRef<str> for InfisicalPath {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl From<InfisicalPath> for String {
    fn from(p: InfisicalPath) -> Self {
        p.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_full_reference() {
        let raw = format!(
            "infisical:///my-secret-key?env=prod&path=/app/backend&project_id={}",
            InfisicalProjectId::from(Uuid::new_v4())
        );
        let reference =
            InfisicalReference::from_str(raw.as_str()).expect("should parse valid reference");

        assert_eq!(reference.key.as_str(), "my-secret-key");

        assert_eq!(reference.options.env.unwrap().as_ref(), "prod");
        assert_eq!(
            reference.options.path,
            Some(InfisicalPath::try_from("/app/backend".to_string()).unwrap())
        );
    }

    #[test]
    fn test_parse_minimal_reference() {
        let raw = "infisical:///simple-key";
        let reference = InfisicalReference::from_str(raw).expect("should parse minimal reference");

        assert_eq!(reference.key.as_str(), "simple-key");
        assert_eq!(reference.options.env, None);
    }

    #[test]
    fn test_url_encoding_handling() {
        let raw = "infisical:///My%20Secret%20Key?env=staging-env";
        let reference = InfisicalReference::from_str(raw).expect("should handle encoding");

        assert_eq!(reference.key.as_str(), "My Secret Key");
        assert_eq!(reference.options.env.unwrap().as_ref(), "staging-env");
    }

    #[test]
    fn test_display_round_trip() {
        let original = InfisicalReference {
            key: InfisicalSecretKey::parse("complex* -_key name").unwrap(),
            options: InfisicalOptions {
                env: Some(InfisicalSlug::try_from("production".to_string()).unwrap()),
                path: Some(InfisicalPath::try_from("/deeply/nested/path".to_string()).unwrap()),
                secret_type: Some(InfisicalSecretType::default()),
                project_id: None,
            },
        };

        let serialized = original.to_string();

        assert!(serialized.starts_with("infisical:///complex*%20-_key%20name"));
        let deserialized = InfisicalReference::from_str(&serialized).expect("should re-parse");
        assert_eq!(original, deserialized);
    }

    #[test]
    fn test_reject_colon() {
        let raw = "infisical:///Key:With:Colon";
        let err = InfisicalReference::from_str(raw);

        assert!(matches!(
            err,
            Err(InfisicalParseError::Validation(ValidationError::Key(_)))
        ));
    }

    #[test]
    fn test_reject_slash_in_key() {
        let raw = "infisical:///folder/key";
        let err = InfisicalReference::from_str(raw);
        assert!(matches!(
            err,
            Err(InfisicalParseError::Validation(ValidationError::Key(_)))
        ));
    }

    #[test]
    fn test_slug_enforcement() {
        assert!(InfisicalSlug::try_from("prod-v1".to_string()).is_ok());
        assert!(InfisicalSlug::try_from("Prod".to_string()).is_err());

        let raw = "infisical:///key?env=Bad_Slug";
        let res = InfisicalReference::from_str(raw);

        assert!(res.is_err());
    }

    #[test]
    fn test_path_validation() {
        assert!(InfisicalPath::try_from("/prod/backend-service/v1".to_string()).is_ok());
        assert!(InfisicalPath::try_from("/TEST_AREA/my_folder".to_string()).is_ok());
        let err = InfisicalPath::try_from("/prod/my folder".to_string()).unwrap_err();
        assert!(matches!(err, ValidationError::Path(_)));

        let err = InfisicalPath::try_from("/prod/v1.0".to_string()).unwrap_err();
        assert!(matches!(err, ValidationError::Path(_)));

        let err = InfisicalPath::try_from("prod/db".to_string()).unwrap_err();
        assert!(matches!(err, ValidationError::Path(_)));
    }

    #[test]
    fn test_project_id_validation() {
        assert!(InfisicalProjectId::try_from(Uuid::new_v4().to_string()).is_ok());
        let err = InfisicalProjectId::try_from("invalid-uuid".to_string()).unwrap_err();
        assert!(matches!(err, ValidationError::ProjectId(_)));
    }

    #[test]
    fn test_secret_type_validation() {
        let raw = "infisical:///key?type=shared";
        let reference = InfisicalReference::from_str(raw).expect("should parse shared");
        assert_eq!(
            reference.options.secret_type,
            Some(InfisicalSecretType::Shared)
        );

        let raw = "infisical:///key?type=personal";
        let reference = InfisicalReference::from_str(raw).expect("should parse personal");
        assert_eq!(
            reference.options.secret_type,
            Some(InfisicalSecretType::Personal)
        );

        let raw = "infisical:///key?type=user";
        let err = InfisicalReference::from_str(raw);

        assert!(err.is_err());

        assert!(matches!(
            err.unwrap_err(),
            InfisicalParseError::QueryParam(_)
        ));

        let raw = "infisical:///key?type=SHARED";
        assert!(InfisicalReference::from_str(raw).is_err());
    }
}
