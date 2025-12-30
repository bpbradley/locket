#![cfg(feature = "testing")]

use async_trait::async_trait;
use locket::{
    path::{AbsolutePath, CanonicalPath, PathMapping},
    provider::{ProviderError, ReferenceParser, SecretReference, SecretsProvider},
    secrets::{InjectFailurePolicy, SecretError, SecretFileManager, SecretFileOpts},
};
use secrecy::SecretString;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use tempfile::tempdir;

// Holds a static map of "Remote" secrets to serve.
#[derive(Debug, Clone, Default)]
struct MockProvider {
    data: HashMap<String, SecretString>,
}

impl MockProvider {
    fn new(data: Vec<(&str, &str)>) -> Self {
        let mut map = HashMap::new();
        for (k, v) in data {
            map.insert(k.to_string(), SecretString::new(v.into()));
        }
        Self { data: map }
    }
}

// Pure Logic: Only accepts "test:" prefix.
// Generates SecretReference::Mock, avoiding OpReference entirely.
impl ReferenceParser for MockProvider {
    fn parse(&self, raw: &str) -> Option<SecretReference> {
        if raw.starts_with("test:") {
            Some(SecretReference::Mock(raw.to_string()))
        } else {
            None
        }
    }
}

#[async_trait]
impl SecretsProvider for MockProvider {
    async fn fetch_map(
        &self,
        references: &[SecretReference],
    ) -> Result<HashMap<SecretReference, SecretString>, ProviderError> {
        let mut result = HashMap::new();

        for ref_obj in references {
            // Pattern match to extract the inner string from the Mock variant
            if let SecretReference::Mock(key) = ref_obj {
                if let Some(val) = self.data.get(key) {
                    result.insert(ref_obj.clone(), val.clone());
                } else {
                    return Err(ProviderError::NotFound(key.clone()));
                }
            }
            // If we somehow got a BWS/Op variant here, we ignore it
            // (or error, but ignoring is standard provider behavior)
        }
        Ok(result)
    }
}

fn setup(
    tpl_name: &str,
    tpl_content: &str,
) -> (tempfile::TempDir, std::path::PathBuf, SecretFileOpts) {
    let tmp = tempdir().unwrap();
    let tpl_dir = tmp.path().join("templates");
    let out_dir = tmp.path().join("secrets");
    std::fs::create_dir_all(&tpl_dir).unwrap();
    std::fs::create_dir_all(&out_dir).unwrap();

    std::fs::write(tpl_dir.join(tpl_name), tpl_content).unwrap();

    let opts = SecretFileOpts::default()
        .with_mapping(vec![make_mapping(&tpl_dir, &out_dir)])
        .with_secret_dir(AbsolutePath::new(&out_dir));

    (tmp, out_dir, opts)
}

#[tokio::test]
async fn test_happy_path_template_rendering() {
    let (_tmp, out_dir, opts) = setup(
        "config.yaml",
        "user: {{ test:user }}\npass: {{ test:pass }}",
    );

    let provider = Arc::new(MockProvider::new(vec![
        ("test:user", "admin"),
        ("test:pass", "secret123"),
    ]));

    let manager = SecretFileManager::new(opts, provider).unwrap();

    manager.inject_all().await.unwrap();

    let result = std::fs::read_to_string(out_dir.join("config.yaml")).unwrap();
    assert_eq!(result, "user: admin\npass: secret123");
}

#[tokio::test]
async fn test_whole_file_replacement() {
    let (_tmp, out_dir, opts) = setup("id_rsa", "test:ssh/key");

    let key_content = "-----BEGIN RSA PRIVATE KEY-----...";
    let provider = Arc::new(MockProvider::new(vec![("test:ssh/key", key_content)]));
    let manager = SecretFileManager::new(opts, provider).unwrap();

    manager.inject_all().await.unwrap();

    let result = std::fs::read_to_string(out_dir.join("id_rsa")).unwrap();
    assert_eq!(result, key_content);
}

#[tokio::test]
async fn test_policy_error_aborts() {
    // "test:missing" parses as valid, but is not in the provider's data map.
    let (_tmp, _out, mut opts) = setup("config.yaml", "Key: {{ test:missing }}");

    opts.policy = InjectFailurePolicy::Error;

    let provider = Arc::new(MockProvider::new(vec![]));
    let manager = SecretFileManager::new(opts, provider).unwrap();

    let result = manager.inject_all().await;

    assert!(result.is_err());

    match result.unwrap_err() {
        SecretError::Provider(ProviderError::NotFound(k)) => {
            assert_eq!(k, "test:missing")
        }
        e => panic!("Unexpected error type: {:?}", e),
    }
}

#[tokio::test]
async fn test_policy_copy_unmodified() {
    let (_tmp, out_dir, mut opts) = setup("config.yaml", "Key: {{ test:missing }}");

    opts.policy = InjectFailurePolicy::CopyUnmodified;

    let provider = Arc::new(MockProvider::new(vec![]));
    let manager = SecretFileManager::new(opts, provider).unwrap();

    manager.inject_all().await.unwrap();

    let result = std::fs::read_to_string(out_dir.join("config.yaml")).unwrap();
    assert_eq!(result, "Key: {{ test:missing }}");
}

#[tokio::test]
async fn test_ignore_unknown_providers() {
    // "test:valid" -> Parsed (starts with test:) -> Fetched
    // "op://real/secret" -> Not Parsed (MockProvider returns None) -> Ignored (Literal)
    let content = "A: {{ op://real/secret }}\nB: {{ test:valid }}";
    let (_tmp, out_dir, opts) = setup("mixed.yaml", content);

    let provider = Arc::new(MockProvider::new(vec![("test:valid", "value")]));
    let manager = SecretFileManager::new(opts, provider).unwrap();

    manager.inject_all().await.unwrap();

    let result = std::fs::read_to_string(out_dir.join("mixed.yaml")).unwrap();

    // The op:// tag should be preserved exactly as is because the provider didn't recognize it
    assert_eq!(result, "A: {{ op://real/secret }}\nB: value");
}

fn make_mapping(src: impl AsRef<Path>, dst: impl AsRef<Path>) -> PathMapping {
    PathMapping::try_new(
        CanonicalPath::try_new(src).expect("test source must exist"),
        AbsolutePath::new(dst),
    )
    .expect("mapping creation failed")
}
