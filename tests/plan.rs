use secret_sidecar::{config::Config, envvars, mirror};
use std::env;

#[test]
fn plan_env_secrets_collects_prefixed_vars() {
    let _g = TestEnv::set_vars(vec![
        ("secret_DbPassword", "op://vault/item/password"),
        ("OTHER", "ignored"),
    ]);
    let cfg = Config::default();
    let plans = envvars::plan_env_secrets(&cfg);
    let names: Vec<_> = plans.iter().map(|p| p.name.clone()).collect();
    assert!(names.contains(&"dbpassword".to_string()));
}

#[test]
fn plan_templates_maps_files() {
    let tmp = tempfile::tempdir().unwrap();
    let tpl = tmp.path().join("templates");
    let out = tmp.path().join("out");
    std::fs::create_dir_all(tpl.join("a/b")).unwrap();
    std::fs::write(tpl.join("a/b/x.txt"), b"hello").unwrap();

    let cfg = Config {
        templates_dir: tpl.clone(),
        output_dir: out.clone(),
        ..Default::default()
    };

    let plans = mirror::plan_templates(&cfg);
    assert_eq!(plans.len(), 1);
    assert!(plans[0].dst.ends_with("a/b/x.txt"));
}

struct TestEnv {
    saved: Vec<(String, Option<String>)>,
}
impl TestEnv {
    fn set_vars(vars: Vec<(&str, &str)>) -> Self {
        let saved = vars
            .iter()
            .map(|(k, _)| {
                let key = (*k).to_string();
                let old = env::var(k).ok();
                unsafe { env::set_var(k, "") };
                (key, old)
            })
            .collect();
        for (k, v) in vars {
            unsafe { env::set_var(k, v) };
        }
        Self { saved }
    }
}
impl Drop for TestEnv {
    fn drop(&mut self) {
        for (k, v) in self.saved.drain(..) {
            match v {
                Some(val) => unsafe { env::set_var(&k, val) },
                None => unsafe { env::remove_var(&k) },
            }
        }
    }
}
