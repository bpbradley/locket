use secret_sidecar::secrets::{
    Secrets,
    manager::{PathMapping, SecretsOpts},
    types::SecretError,
};
use std::{collections::HashMap, vec};

#[test]
fn collisions_structure_conflict() {
    let tmp = tempfile::tempdir().unwrap();
    let templates = tmp.path().join("templates");
    let output = tmp.path().join("out");
    std::fs::create_dir_all(&templates).unwrap();

    let blocker_src = templates.join("config");
    std::fs::write(&blocker_src, "parent").unwrap();

    let blocked_label = "config/db_pass";
    let mut initial_values = HashMap::new();
    initial_values.insert(blocked_label.to_string(), "child".to_string());

    let opts = SecretsOpts::default()
        .with_value_dir(output.clone())
        .with_mapping(vec![PathMapping::new(templates.clone(), output.clone())]);

    let secrets = Secrets::new(opts).with_values(initial_values);

    let result = secrets.collisions();

    assert!(result.is_err());

    assert!(matches!(
        result.unwrap_err(),
        SecretError::StructureConflict { .. }
    ));
}

#[test]
fn collisions_report_rich_exact_collision() {
    let tmp = tempfile::tempdir().unwrap();
    let src_dir = tmp.path().join("src");
    let out_dir = tmp.path().join("out");
    std::fs::create_dir_all(&src_dir).unwrap();

    let file_src = src_dir.join("dup");
    std::fs::write(&file_src, "x").unwrap();

    let mut values = HashMap::new();
    values.insert("dup".to_string(), "y".to_string());

    let opts = SecretsOpts::default()
        .with_value_dir(out_dir.clone())
        .with_mapping(vec![PathMapping::new(src_dir, out_dir.clone())]);

    let secrets = Secrets::new(opts).with_values(values);

    let result = secrets.collisions();

    assert!(result.is_err());

    assert!(matches!(result.unwrap_err(), SecretError::Collision { .. }));
}
