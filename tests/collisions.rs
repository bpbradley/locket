use locket::secrets::{PathMapping, SecretError, Secrets, SecretsOpts};
use std::{collections::HashMap, vec};

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

    let opts = SecretsOpts::default().with_mapping(vec![
        PathMapping::new(blocker_src.clone(), output.join("config")),
        PathMapping::new(blocked_src.clone(), output.join("config/nested")),
    ]);

    let secrets = Secrets::new(opts);

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

    let opts = SecretsOpts::default()
        .with_value_dir(out_dir.clone())
        .with_mapping(vec![PathMapping::new(src_dir, out_dir.clone())]);

    let secrets = Secrets::new(opts).with_values(values);

    let result = secrets.collisions();

    assert!(result.is_err());

    assert!(matches!(result.unwrap_err(), SecretError::Collision { .. }));
}
