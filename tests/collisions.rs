use secret_sidecar::secrets::{
    Secrets,
    manager::{PathMapping, SecretsOpts},
    types::SecretError,
};
use std::collections::HashMap;

#[test]
fn collisions_detect_duplicate_dst_across_files_and_values() {
    let tmp = tempfile::tempdir().unwrap();
    let templates = tmp.path().join("templates");
    let output = tmp.path().join("out");

    std::fs::create_dir_all(&templates).unwrap();
    std::fs::write(templates.join("dup.txt"), b"x").unwrap();

    let mut initial_values = HashMap::new();
    initial_values.insert("dup.txt".to_string(), "some template value".to_string());
    let opts = SecretsOpts::new()
        .with_value_dir(output.clone())
        .with_mapping(vec![PathMapping::new(templates.clone(), output.clone())]);

    let mut secrets = Secrets::new(opts);
    secrets.extend_values(initial_values);

    let result = secrets.collisions();

    assert!(
        result.is_err(),
        "Should detect collision between file 'dup.txt' and value 'dup.txt'"
    );

    match result.unwrap_err() {
        SecretError::Config(msg) => {
            assert!(msg.contains("Collision"));
            assert!(msg.contains("dup.txt"));
        }
        _ => panic!("Expected Config error"),
    }
}

#[test]
fn collisions_detect_structure_conflict_file_blocking_dir() {
    let tmp = tempfile::tempdir().unwrap();
    let templates = tmp.path().join("templates");
    let output = tmp.path().join("out");

    std::fs::create_dir_all(&templates).unwrap();

    std::fs::write(templates.join("app_config"), b"file content").unwrap();

    let mut initial_values = HashMap::new();
    initial_values.insert("app_config/db_pass".to_string(), "secret".to_string());

    let opts = SecretsOpts::new()
        .with_value_dir(output.clone())
        .with_mapping(vec![PathMapping::new(templates.clone(), output.clone())]);

    let mut secrets = Secrets::new(opts);
    secrets.extend_values(initial_values);

    // 3. Validate
    let result = secrets.collisions();

    // 4. Assert
    assert!(result.is_err(), "Should detect structure conflict");

    match result.unwrap_err() {
        SecretError::Config(msg) => {
            // Check that the error message explains the hierarchy issue
            assert!(msg.contains("Structure Conflict"));
            assert!(msg.contains("app_config"));
        }
        _ => panic!("Expected Config error"),
    }
}
