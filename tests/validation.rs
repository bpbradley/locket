use locket::path::{AbsolutePath, CanonicalPath, PathMapping};
use locket::provider::{ProviderError, SecretsProvider};
use locket::secrets::{Secret, SecretError, SecretFileManager, SecretFileOpts};
use secrecy::SecretString;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use tempfile::tempdir;

struct NoOpProvider;
#[async_trait::async_trait]
impl SecretsProvider for NoOpProvider {
    fn accepts_key(&self, _key: &str) -> bool {
        true
    }
    async fn fetch_map(
        &self,
        _references: &[&str],
    ) -> Result<HashMap<String, SecretString>, ProviderError> {
        Ok(HashMap::new())
    }
}

#[test]
fn collisions_structure_conflict() {
    let tmp = tempfile::tempdir().unwrap();

    // Create distinct source directories so we don't conflict on the input side
    let src_a = tmp.path().join("src_a");
    let src_b = tmp.path().join("src_b");
    let output = tmp.path().join("out");

    std::fs::create_dir_all(&src_a).unwrap();
    std::fs::create_dir_all(&src_b).unwrap();

    // Maps to "/out/config" (File `config`)
    let blocker_src = src_a.join("config");
    std::fs::write(&blocker_src, "I am a file").unwrap();

    // Maps to "/out/config/nested" (Ambiguous directory `config`)
    let blocked_src = src_b.join("nested");
    std::fs::write(&blocked_src, "I am inside a dir").unwrap();

    let opts = SecretFileOpts::default().with_mapping(vec![
        make_mapping(blocker_src.clone(), output.join("config")),
        make_mapping(blocked_src.clone(), output.join("config/nested")),
    ]);

    let secrets = SecretFileManager::new(opts, Arc::new(NoOpProvider)).unwrap();

    let result = secrets.collisions();

    assert!(result.is_err(), "Should detect structure conflict");
    assert!(matches!(
        result.unwrap_err(),
        SecretError::StructureConflict { .. }
    ));
}

#[test]
fn collisions_on_output_dst() {
    let tmp = tempfile::tempdir().unwrap();
    let src_dir = tmp.path().join("src");
    let out_dir = tmp.path().join("out");
    std::fs::create_dir_all(&src_dir).unwrap();

    let file_src = src_dir.join("dup");
    std::fs::write(&file_src, "x").unwrap();

    let mut values = HashMap::new();
    values.insert("dup".to_string(), "y".to_string());

    let args: Vec<Secret> = Secret::try_from_map(values.clone()).unwrap();

    let opts = SecretFileOpts::default()
        .with_secret_dir(AbsolutePath::new(&out_dir))
        .with_mapping(vec![make_mapping(&src_dir, &out_dir)])
        .with_secrets(args);

    let manager = SecretFileManager::new(opts, Arc::new(NoOpProvider)).unwrap();

    let result = manager.collisions();

    assert!(result.is_err());

    assert!(matches!(result.unwrap_err(), SecretError::Collision { .. }));
}

#[test]
fn validate_fails_loop_dst_inside_src() {
    let tmp = tempdir().unwrap();
    let src = tmp.path().join("templates");
    let dst = src.join("nested_out");

    std::fs::create_dir_all(&src).unwrap();

    let opts = SecretFileOpts::default().with_mapping(vec![make_mapping(&src, &dst)]);
    let manager = SecretFileManager::new(opts.clone(), Arc::new(NoOpProvider));
    assert!(matches!(
        manager,
        Err(SecretError::Loop { src: s, dst: d }) if s == src && d == dst
    ));
}

#[test]
fn validate_fails_destructive() {
    let tmp = tempdir().unwrap();
    let dst = tmp.path().join("out");
    let src = dst.join("templates");

    std::fs::create_dir_all(&src).unwrap();

    let opts = SecretFileOpts::default().with_mapping(vec![make_mapping(&src, &dst)]);
    let manager = SecretFileManager::new(opts.clone(), Arc::new(NoOpProvider));
    assert!(matches!(
        manager,
        Err(SecretError::Destructive { src: s, dst: d }) if s == src && d == dst
    ));
}

#[test]
fn validate_fails_value_dir_loop() {
    let tmp = tempdir().unwrap();
    let src = tmp.path().join("templates");
    std::fs::create_dir_all(&src).unwrap();

    let dst = tmp.path().join("safe_out");
    let bad_value_dir = src.join("values");

    let opts = SecretFileOpts::default()
        .with_mapping(vec![make_mapping(&src, &dst)])
        .with_secret_dir(AbsolutePath::new(&bad_value_dir));

    let manager = SecretFileManager::new(opts, Arc::new(NoOpProvider));

    assert!(matches!(
        manager,
        Err(SecretError::Loop { src: s, dst: d }) if s == src && d == bad_value_dir
    ));
}

#[test]
fn validate_succeeds_valid_config() {
    let tmp = tempdir().unwrap();
    let src = tmp.path().join("templates");
    let dst = tmp.path().join("out");

    std::fs::create_dir_all(&src).unwrap();

    let opts = SecretFileOpts::default().with_mapping(vec![make_mapping(&src, &dst)]);
    let manager = SecretFileManager::new(opts, Arc::new(NoOpProvider));

    assert!(manager.is_ok());
}

fn make_mapping(src: impl AsRef<Path>, dst: impl AsRef<Path>) -> PathMapping {
    PathMapping::try_new(
        CanonicalPath::try_new(src).expect("test source must exist"),
        AbsolutePath::new(dst),
    )
    .expect("mapping creation failed")
}
