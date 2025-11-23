use secret_sidecar::{
    provider::{ProviderError, SecretsProvider},
    secrets::{
        SecretError, Secrets,
        manager::{PathMapping, SecretsOpts},
    },
};
use std::env;
use std::path::Path;

#[derive(Clone, Default)]
struct MockProvider {
    inject_should_fail: bool,
}

impl SecretsProvider for MockProvider {
    fn inject(&self, src: &Path, dst: &Path) -> Result<(), ProviderError> {
        if self.inject_should_fail {
            return Err(ProviderError::Other("inject failed (mock)".into()));
        }
        let data = std::fs::read(src).map_err(ProviderError::Io)?;
        std::fs::write(dst, data).map_err(ProviderError::Io)?;
        Ok(())
    }
}

#[test]
fn inject_all_success_for_files_and_values() {
    let tmp = tempfile::tempdir().unwrap();
    let tpl = tmp.path().join("templates");
    std::fs::create_dir_all(&tpl).unwrap();
    std::fs::write(tpl.join("a.txt"), b"hello").unwrap();
    let out = tmp.path().join("out");
    let opts = SecretsOpts::default()
        .with_value_dir(out.clone())
        .with_mapping(vec![PathMapping::new(tpl.clone(), out.clone())]);
    let mut secrets = Secrets::new(opts);

    secrets.add_value("Greeting", "Hi {{name}}");

    let provider = MockProvider::default();
    secrets.inject_all(&provider).unwrap();
    let got_file = std::fs::read(out.join("a.txt")).unwrap();
    assert_eq!(got_file, b"hello");
    let got_value = std::fs::read(out.join("greeting")).unwrap();
    assert_eq!(got_value, b"Hi {{name}}");
}

#[test]
fn inject_all_fallback_copy_on_error() {
    let tmp = tempfile::tempdir().unwrap();
    let tpl = tmp.path().join("templates");
    std::fs::create_dir_all(&tpl).unwrap();
    std::fs::write(tpl.join("bin.dat"), b"RAW").unwrap();
    let out = tmp.path().join("out");
    let opts = SecretsOpts::default()
        .with_value_dir(out.clone())
        .with_mapping(vec![PathMapping::new(tpl.clone(), out.clone())]);
    let secrets = Secrets::new(opts);

    let provider = MockProvider {
        inject_should_fail: true,
    };
    secrets.inject_all(&provider).unwrap();
    let got = std::fs::read(out.join("bin.dat")).unwrap();
    assert_eq!(got, b"RAW");
}

#[test]
fn inject_all_error_without_fallback() {
    let tmp = tempfile::tempdir().unwrap();
    let tpl = tmp.path().join("templates");
    std::fs::create_dir_all(&tpl).unwrap();
    std::fs::write(tpl.join("bin.dat"), b"X").unwrap();
    let out = tmp.path().join("out");
    let opts = SecretsOpts::default()
        .with_value_dir(out.clone())
        .with_mapping(vec![PathMapping::new(tpl.clone(), out.clone())])
        .with_policy(secret_sidecar::secrets::InjectFailurePolicy::Error);
    let secrets = Secrets::new(opts);
    let provider = MockProvider {
        inject_should_fail: true,
    };
    let err = secrets.inject_all(&provider).unwrap_err();
    match err {
        SecretError::InjectionFailed { .. } => { /* expected variant */ }
        other => panic!("expected InjectionFailed, got: {other}"),
    }
}

#[test]
fn inject_all_value_sources() {
    let _g = TestEnv::set_vars(vec![("secret_GREETING", "Hello {{name}}!")]);
    let tmp = tempfile::tempdir().unwrap();
    let out = tmp.path().join("out");
    let opts = SecretsOpts::default()
        .with_value_dir(out.clone())
        .with_mapping(vec![PathMapping::new(tmp.path().join("templates"), out.clone())])
        .with_env_value_prefix("secret_");
    let secrets = Secrets::new(opts);
    let provider = MockProvider::default();
    secrets.inject_all(&provider).unwrap();
    let got = std::fs::read(out.join("greeting")).unwrap();
    assert_eq!(got, b"Hello {{name}}!");
}

struct TestEnv {
    saved: Vec<(String, Option<String>)>,
}
impl TestEnv {
    fn set_vars(vars: Vec<(&str, &str)>) -> Self {
        let keys: Vec<String> = vars.iter().map(|(k, _)| k.to_string()).collect();
        let mut saved = Vec::new();
        // save any existing
        for k in &keys {
            saved.push((k.clone(), env::var(k).ok()));
        }
        for (k, _) in env::vars() {
            if k.starts_with("secret_") {
                unsafe { env::remove_var(k) };
            }
        }
        // set requested
        for (k, v) in vars {
            unsafe { env::set_var(k, v) };
        }
        Self { saved }
    }
}
impl Drop for TestEnv {
    fn drop(&mut self) {
        for (k, _) in env::vars() {
            if k.starts_with("secret_") {
                unsafe { env::remove_var(k) };
            }
        }
        // restore saved
        for (k, v) in self.saved.drain(..) {
            match v {
                Some(val) => unsafe { env::set_var(&k, val) },
                None => unsafe { env::remove_var(&k) },
            }
        }
    }
}
