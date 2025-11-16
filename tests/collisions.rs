use secret_sidecar::secrets::{InjectFailurePolicy, Secrets, SecretsOpts};
use std::fs;

#[test]
fn collisions_detect_duplicate_dst_across_files_and_values() {
    let tmp = tempfile::tempdir().unwrap();
    let templates = tmp.path().join("templates");
    let output = tmp.path().join("out");
    fs::create_dir_all(&templates).unwrap();
    // create a template file that will map to out/dup.txt
    fs::write(templates.join("dup.txt"), b"x").unwrap();
    let mut s = Secrets::new(SecretsOpts {
        templates_root: templates.clone(),
        output_root: output.clone(),
        policy: InjectFailurePolicy::CopyUnmodified,
        ..Default::default()
    })
    .collect();
    // Add a value that sanitizes to same dst path
    s.add_value("dup.txt", "template");
    let expected_dst = output.join("dup.txt");
    let cols = s.collisions();
    assert!(cols.iter().any(|p| p == &expected_dst));
}
