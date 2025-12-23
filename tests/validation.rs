use locket::path::{AbsolutePath, CanonicalPath, PathMapping};
use locket::secrets::{SecretError, SecretFileOpts};
use std::path::Path;
use tempfile::tempdir;

#[test]
fn validate_fails_loop_dst_inside_src() {
    let tmp = tempdir().unwrap();
    let src = tmp.path().join("templates");
    let dst = src.join("nested_out");

    std::fs::create_dir_all(&src).unwrap();

    let mut opts = SecretFileOpts::default().with_mapping(vec![make_mapping(&src, &dst)]);

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

    let mut opts = SecretFileOpts::default().with_mapping(vec![make_mapping(&src, &dst)]);

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

    let mut opts = SecretFileOpts::default()
        .with_mapping(vec![make_mapping(&src, &dst)])
        .with_secret_dir(AbsolutePath::new(&bad_value_dir));

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

    let mut opts = SecretFileOpts::default().with_mapping(vec![make_mapping(&src, &dst)]);

    assert!(opts.resolve().is_ok());
}

fn make_mapping(src: impl AsRef<Path>, dst: impl AsRef<Path>) -> PathMapping {
    PathMapping::try_new(
        CanonicalPath::try_new(src).expect("test source must exist"),
        AbsolutePath::new(dst),
    )
    .expect("mapping creation failed")
}
