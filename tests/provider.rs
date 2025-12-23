use async_trait::async_trait;
use locket::{
    path::{AbsolutePath, CanonicalPath, PathMapping},
    provider::{ProviderError, SecretsProvider},
    secrets::{InjectFailurePolicy, SecretError, SecretFileManager, SecretFileOpts},
};
use secrecy::SecretString;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use tempfile::tempdir;
use tracing::debug;

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

#[async_trait]
impl SecretsProvider for MockProvider {
    fn accepts_key(&self, key: &str) -> bool {
        key.starts_with("mock://")
    }

    async fn fetch_map(
        &self,
        references: &[&str],
    ) -> Result<HashMap<String, SecretString>, ProviderError> {
        let mut result = HashMap::new();
        for &key in references {
            if let Some(val) = self.data.get(key) {
                result.insert(key.to_string(), val.clone());
            } else {
                return Err(ProviderError::NotFound(key.to_string()));
            }
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
    // A file with two secrets
    let (_tmp, out_dir, opts) = setup(
        "config.yaml",
        "user: {{ mock://user }}\npass: {{ mock://pass }}",
    );

    // Provider has both secrets
    let provider = Arc::new(MockProvider::new(vec![
        ("mock://user", "admin"),
        ("mock://pass", "secret123"),
    ]));

    let manager = SecretFileManager::new(opts, provider).unwrap();

    manager.inject_all().await.unwrap();

    let result = std::fs::read_to_string(out_dir.join("config.yaml")).unwrap();
    assert_eq!(result, "user: admin\npass: secret123");
}

#[tokio::test]
async fn test_whole_file_replacement() {
    let (_tmp, out_dir, opts) = setup("id_rsa", "mock://ssh/key");

    let key_content = "-----BEGIN RSA PRIVATE KEY-----...";
    let provider = Arc::new(MockProvider::new(vec![("mock://ssh/key", key_content)]));
    let manager = SecretFileManager::new(opts, provider).unwrap();

    manager.inject_all().await.unwrap();

    let result = std::fs::read_to_string(out_dir.join("id_rsa")).unwrap();
    assert_eq!(result, key_content); // Should be replaced entirely
}

#[tokio::test]
async fn test_policy_error_aborts() {
    // Template requests a missing secret
    let (_tmp, _out, mut opts) = setup("config.yaml", "Key: {{ mock://missing }}");

    opts.policy = InjectFailurePolicy::Error;

    let provider = Arc::new(MockProvider::new(vec![])); // Empty provider
    let manager = SecretFileManager::new(opts, provider).unwrap();

    let result = manager.inject_all().await;

    assert!(result.is_err());

    match result.unwrap_err() {
        SecretError::Provider(ProviderError::NotFound(k)) => assert_eq!(k, "mock://missing"),
        e => panic!("Unexpected error type: {:?}", e),
    }
}

#[tokio::test]
async fn test_policy_copy_unmodified() {
    // Template requests missing secret
    let (_tmp, out_dir, mut opts) = setup("config.yaml", "Key: {{ mock://missing }}");

    opts.policy = InjectFailurePolicy::CopyUnmodified;

    let provider = Arc::new(MockProvider::new(vec![]));
    let manager = SecretFileManager::new(opts, provider).unwrap();

    // Should succeed (return Ok) despite missing secret
    manager.inject_all().await.unwrap();

    // Should contain original template text
    let result = std::fs::read_to_string(out_dir.join("config.yaml")).unwrap();
    assert_eq!(result, "Key: {{ mock://missing }}");
}

#[tokio::test]
async fn test_ignore_unknown_providers() {
    // File contains keys for a different provider (e.g. op://)
    // Our mock only accepts mock://
    let content = "A: {{ op://vault/item }}\nB: {{ mock://valid }}";
    let (_tmp, out_dir, opts) = setup("mixed.yaml", content);

    let provider = Arc::new(MockProvider::new(vec![("mock://valid", "value")]));
    let manager = SecretFileManager::new(opts, provider).unwrap();

    manager.inject_all().await.unwrap();

    let result = std::fs::read_to_string(out_dir.join("mixed.yaml")).unwrap();

    // "op://" should be ignored (passed through) because accepts_key returned false
    // "mock://" should be rendered
    debug!("Result: {}", result);
    assert_eq!(result, "A: {{ op://vault/item }}\nB: value");
}

fn make_mapping(src: impl AsRef<Path>, dst: impl AsRef<Path>) -> PathMapping {
    PathMapping::try_new(
        CanonicalPath::try_new(src).expect("test source must exist"),
        AbsolutePath::new(dst),
    )
    .expect("mapping creation failed")
}
