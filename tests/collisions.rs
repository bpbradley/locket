use secret_sidecar::secrets::{FileSource, Secrets};
use std::path::PathBuf;

#[test]
fn collisions_detect_duplicate_dst_across_files_and_values() {
    let templates_root = PathBuf::from("/templates");
    let output_root = PathBuf::from("/out");
    let mut s = Secrets::new(templates_root.clone(), output_root.clone());
    // Simulate file mapping: create a FileSource manually (no actual FS needed for collision logic)
    let src1 = templates_root.join("dup.txt");
    let file_fs = FileSource::from_src(&templates_root, &output_root, src1.clone()).unwrap();
    s.upsert_file(src1.clone());
    // Add a value that sanitizes to same dst path
    s.add_value("dup.txt", "template");
    let expected_dst = file_fs.dst.clone();
    let cols = s.collisions();
    assert!(cols.iter().any(|p| p == &expected_dst));
}
