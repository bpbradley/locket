use secret_sidecar::{
    config::Config,
    envvars, mirror,
    provider::{ProviderError, SecretsProvider},
};
use std::env;

#[derive(Clone, Default)]
struct MockProvider {
    inject_should_fail: bool,
}

impl SecretsProvider for MockProvider {
    fn inject(&self, src: &str, dst: &str) -> Result<(), ProviderError> {
        if self.inject_should_fail {
            return Err(ProviderError::Failed("inject failed (mock)".into()));
        }
        let data = std::fs::read(src).map_err(|e| ProviderError::Failed(e.to_string()))?;
        std::fs::write(dst, data).map_err(|e| ProviderError::Failed(e.to_string()))?;
        Ok(())
    }
}

#[test]
fn mirror_inject_success() {
    let tmp = tempfile::tempdir().unwrap();
    let tpl = tmp.path().join("templates");
    let out = tmp.path().join("out");
    std::fs::create_dir_all(&tpl).unwrap();
    std::fs::write(tpl.join("a.txt"), b"hello").unwrap();

    let mut cfg = Config::default();
    cfg.templates_dir = tpl.to_string_lossy().into_owned();
    cfg.output_dir = out.to_string_lossy().into_owned();
    cfg.inject_fallback_copy = true;

    let provider = MockProvider::default();
    mirror::sync_templates(&cfg, &provider).unwrap();

    let got = std::fs::read(out.join("a.txt")).unwrap();
    assert_eq!(got, b"hello");
}

#[test]
fn mirror_inject_failure_fallback_copy() {
    let tmp = tempfile::tempdir().unwrap();
    let tpl = tmp.path().join("templates");
    let out = tmp.path().join("out");
    std::fs::create_dir_all(&tpl).unwrap();
    std::fs::write(tpl.join("bin.dat"), b"RAW-BYTES").unwrap();

    let mut cfg = Config::default();
    cfg.templates_dir = tpl.to_string_lossy().into_owned();
    cfg.output_dir = out.to_string_lossy().into_owned();
    cfg.inject_fallback_copy = true;

    let provider = MockProvider {
        inject_should_fail: true,
    };
    mirror::sync_templates(&cfg, &provider).unwrap();

    let got = std::fs::read(out.join("bin.dat")).unwrap();
    assert_eq!(got, b"RAW-BYTES");
}

#[test]
fn mirror_inject_failure_no_fallback_is_error() {
    let tmp = tempfile::tempdir().unwrap();
    let tpl = tmp.path().join("templates");
    let out = tmp.path().join("out");
    std::fs::create_dir_all(&tpl).unwrap();
    std::fs::write(tpl.join("bin.dat"), b"X").unwrap();

    let mut cfg = Config::default();
    cfg.templates_dir = tpl.to_string_lossy().into_owned();
    cfg.output_dir = out.to_string_lossy().into_owned();
    cfg.inject_fallback_copy = false;

    let provider = MockProvider {
        inject_should_fail: true,
    };
    let err = mirror::sync_templates(&cfg, &provider).unwrap_err();
    let msg = format!("{}", err);
    assert!(msg.contains("fallback disabled"));
}

#[test]
fn env_inline_template_uses_inject() {
    let _g = TestEnv::set_vars(vec![("secret_GREETING", "Hello {{name}}!")]);
    let tmp = tempfile::tempdir().unwrap();
    let mut cfg = Config::default();
    cfg.output_dir = tmp.path().to_string_lossy().into_owned();

    let provider = MockProvider::default();
    envvars::sync_env_secrets(&cfg, &provider).unwrap();

    let got = std::fs::read(tmp.path().join("greeting")).unwrap();
    // Mock inject copies template bytes directly
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
        // clear everything starting with secret_ to avoid interference
        for (k, _) in env::vars() {
            if k.starts_with("secret_") {
                env::remove_var(k);
            }
        }
        // set requested
        for (k, v) in vars {
            env::set_var(k, v);
        }
        Self { saved }
    }
}
impl Drop for TestEnv {
    fn drop(&mut self) {
        // remove all secret_ vars set during test
        for (k, _) in env::vars() {
            if k.starts_with("secret_") {
                env::remove_var(k);
            }
        }
        // restore saved
        for (k, v) in self.saved.drain(..) {
            match v {
                Some(val) => env::set_var(&k, val),
                None => env::remove_var(&k),
            }
        }
    }
}
