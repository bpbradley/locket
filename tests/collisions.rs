use secret_sidecar::secrets::{FileSource, InjectFailurePolicy, Secrets, SecretsOpts};
use std::path::PathBuf;

#[test]
fn collisions_detect_duplicate_dst_across_files_and_values() {
    let templates = PathBuf::from("/templates");
    let output = PathBuf::from("/output");
    let mut s = Secrets::new(SecretsOpts {
        templates_root: templates.clone(),
        output_root: output.clone(),
        policy: InjectFailurePolicy::CopyUnmodified,
        ..Default::default()
    })
    .collect();
    // Simulate file mapping: create a FileSource manually (no actual FS needed for collision logic)
    let src1 = templates.join("dup.txt");
    let file_fs = FileSource::from_src(&templates, &output, src1.clone()).unwrap();
    s.upsert_file(src1.clone());
    // Add a value that sanitizes to same dst
    s.add_value("dup.txt", "template");
    let expected_dst = file_fs.dst.clone();
    let cols = s.collisions();
    assert!(cols.iter().any(|p| p == &expected_dst));
}
