use locket::path::{AbsolutePath, CanonicalPath, PathMapping};
use locket::provider::{ProviderError, ReferenceParser, SecretReference, SecretsProvider};
use locket::secrets::{Secret, SecretError, SecretFileManager, SecretManagerConfig};
use secrecy::SecretString;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use tempfile::tempdir;

struct NoOpProvider;

impl ReferenceParser for NoOpProvider {
    fn parse(&self, _raw: &str) -> Option<SecretReference> {
        None
    }
}

#[async_trait::async_trait]
impl SecretsProvider for NoOpProvider {
    async fn fetch_map(
        &self,
        _references: &[SecretReference],
    ) -> Result<HashMap<SecretReference, SecretString>, ProviderError> {
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

    let config = SecretManagerConfig {
        map: vec![
            make_mapping(blocker_src.clone(), output.join("config")),
            make_mapping(blocked_src.clone(), output.join("config/nested")),
        ],
        ..Default::default()
    };

    let secrets = SecretFileManager::new(config, Arc::new(NoOpProvider));

    assert!(secrets.is_err(), "Should detect structure conflict");
    assert!(matches!(
        secrets,
        Err(SecretError::StructureConflict { .. })
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

    let config = SecretManagerConfig {
        out: AbsolutePath::new(&out_dir),
        map: vec![make_mapping(&src_dir, &out_dir)],
        secrets: args,
        ..Default::default()
    };

    let manager = SecretFileManager::new(config, Arc::new(NoOpProvider));

    assert!(manager.is_err());

    assert!(matches!(manager, Err(SecretError::Collision { .. })));
}

#[test]
fn validate_fails_loop_dst_inside_src() {
    let tmp = tempdir().unwrap();
    let src = tmp.path().join("templates");
    let dst = src.join("nested_out");

    std::fs::create_dir_all(&src).unwrap();

    let config = SecretManagerConfig {
        map: vec![make_mapping(&src, &dst)],
        ..Default::default()
    };

    let manager = SecretFileManager::new(config, Arc::new(NoOpProvider));
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

    let config = SecretManagerConfig {
        map: vec![make_mapping(&src, &dst)],
        ..Default::default()
    };

    let manager = SecretFileManager::new(config, Arc::new(NoOpProvider));
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

    let config = SecretManagerConfig {
        map: vec![make_mapping(&src, &dst)],
        out: AbsolutePath::new(&bad_value_dir),
        ..Default::default()
    };

    let manager = SecretFileManager::new(config, Arc::new(NoOpProvider));

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

    let config = SecretManagerConfig {
        map: vec![make_mapping(&src, &dst)],
        ..Default::default()
    };
    let manager = SecretFileManager::new(config, Arc::new(NoOpProvider));

    assert!(manager.is_ok());
}

fn make_mapping(src: impl AsRef<Path>, dst: impl AsRef<Path>) -> PathMapping {
    PathMapping::try_new(
        CanonicalPath::try_new(src).expect("test source must exist"),
        AbsolutePath::new(dst),
    )
    .expect("mapping creation failed")
}
