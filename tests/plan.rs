use secret_sidecar::secrets::{collect_files_iter, collect_value_sources};

#[test]
fn collect_files_maps_tree() {
    let tmp = tempfile::tempdir().unwrap();
    let tpl = tmp.path().join("templates");
    std::fs::create_dir_all(tpl.join("a/b")).unwrap();
    std::fs::write(tpl.join("a/b/x.txt"), b"hello").unwrap();
    std::fs::write(tpl.join("root.txt"), b"hi").unwrap();
    let out = tmp.path().join("out");
    let files: Vec<_> = collect_files_iter(&tpl, &out).collect();
    assert_eq!(files.len(), 2);
    assert!(files.iter().any(|f| f.dst.ends_with("a/b/x.txt")));
    assert!(files.iter().any(|f| f.dst.ends_with("root.txt")));
}

#[test]
fn collect_value_sources_filters_and_sanitizes() {
    let tmp = tempfile::tempdir().unwrap();
    let out = tmp.path();
    let env_like = vec![
        ("secret_DB-PASS", "op://vault/db/password"),
        ("other", "ignored"),
        ("secret_cfg", "inline={{token}}"),
    ];
    // Simulate the env collector stripping prefix prior to generic collector
    let stripped = env_like
        .into_iter()
        .filter_map(|(k, v)| k.strip_prefix("secret_").map(|rest| (rest.to_string(), v)));
    let vals = collect_value_sources(out, stripped);
    let labels: Vec<_> = vals.iter().map(|v| v.label.clone()).collect();
    assert!(labels.contains(&"db-pass".to_string()));
    assert!(labels.contains(&"cfg".to_string()));
    assert_eq!(vals.len(), 2);
}
