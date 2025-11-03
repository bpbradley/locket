use secret_sidecar::secrets::{Secrets, value_source};
use std::path::PathBuf;

#[test]
fn collisions_detect_duplicate_dst_across_files_and_values() {
    let mut s = Secrets::new();
    // Simulate file mapping
    let out_root = PathBuf::from("/out");
    let src1 = PathBuf::from("/src/a.txt");
    let dst = out_root.join("dup.txt");
    s.files.insert(src1, dst.clone());
    // Value with label "dup.txt" sanitized should produce same dst, so use exact label
    s.values
        .push(value_source(out_root.as_path(), "dup.txt", "template"));
    let cols = s.collisions();
    assert!(cols.iter().any(|p| p == &dst));
}
