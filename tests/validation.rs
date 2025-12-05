use locket::secrets::{PathMapping, SecretError, SecretsOpts};
use tempfile::tempdir;

#[test]
fn validate_fails_source_missing() {
    let tmp = tempdir().unwrap();
    let missing_src = tmp.path().join("ghost");
    let dst = tmp.path().join("out");

    let mut opts = SecretsOpts::default().with_mapping(vec![PathMapping::new(&missing_src, &dst)]);
    assert!(matches!(
        opts.resolve(),
        Err(SecretError::SourceMissing(p)) if p == missing_src
    ));
}

#[test]
fn validate_relative_paths_are_canonicalized() {
    let tmp = tempdir().unwrap();
    let src = tmp.path().join("templates");
    std::fs::create_dir_all(&src).unwrap();
    let relative = src.join("..").join("templates");

    let mut opts = SecretsOpts::default().with_mapping(vec![PathMapping::new(&relative, "out")]);
    assert!(opts.resolve().is_ok());
    // Verify it resolved to the absolute path
    assert_eq!(opts.mapping[0].src(), src.as_path());
}

#[test]
fn validate_fails_loop_dst_inside_src() {
    let tmp = tempdir().unwrap();
    let src = tmp.path().join("templates");
    let dst = src.join("nested_out");

    std::fs::create_dir_all(&src).unwrap();

    let mut opts = SecretsOpts::default().with_mapping(vec![PathMapping::new(&src, &dst)]);

    assert!(matches!(
        opts.resolve(),
        Err(SecretError::Loop { src: s, dst: d }) if s == src && d == dst
    ));
}

#[test]
fn validate_fails_destructive() {
    let tmp = tempdir().unwrap();
    let dst = tmp.path().join("out");
    let src = dst.join("templates");

    std::fs::create_dir_all(&src).unwrap();

    let mut opts = SecretsOpts::default().with_mapping(vec![PathMapping::new(&src, &dst)]);

    assert!(matches!(
        opts.resolve(),
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

    let mut opts = SecretsOpts::default()
        .with_mapping(vec![PathMapping::new(&src, &dst)])
        .with_value_dir(bad_value_dir.clone());

    assert!(matches!(
        opts.resolve(),
        Err(SecretError::Loop { src: s, dst: d }) if s == src && d == bad_value_dir
    ));
}

#[test]
fn validate_succeeds_valid_config() {
    let tmp = tempdir().unwrap();
    let src = tmp.path().join("templates");
    let dst = tmp.path().join("out");

    std::fs::create_dir_all(&src).unwrap();

    let mut opts = SecretsOpts::default().with_mapping(vec![PathMapping::new(src, dst)]);

    assert!(opts.resolve().is_ok());
}