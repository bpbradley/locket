use secret_sidecar::secrets::types::sanitize_name;

#[test]
fn sanitize_basic() {
    assert_eq!(sanitize_name("Db_Password"), "db_password");
    assert_eq!(sanitize_name("A/B/C"), "a/b/c");
    assert_eq!(sanitize_name("weird name"), "weird_name");
}

#[test]
fn sanitize_unicode_and_symbols() {
    assert_eq!(sanitize_name("πß?%"), "____");
    assert_eq!(sanitize_name("..//--__"), "..//--__");
}
