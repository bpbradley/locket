use anyhow::Context;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub templates_dir: String,
    pub output_dir: String,
    pub status_file: String,
    pub watch: bool,
    pub inject_fallback_copy: bool,
    pub provider: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            templates_dir: "/templates".into(),
            output_dir: "/run/secrets".into(),
            status_file: "/tmp/.secret-sidecar/ready".into(),
            watch: true,
            inject_fallback_copy: true,
            provider: "op".into(),
        }
    }
}

impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        let mut cfg = Self::default();
        if let Ok(v) = std::env::var("TEMPLATES_DIR") {
            cfg.templates_dir = v;
        }
        if let Ok(v) = std::env::var("OUTPUT_DIR") {
            cfg.output_dir = v;
        }
        if let Ok(v) = std::env::var("STATUS_FILE") {
            cfg.status_file = v;
        }
        if let Ok(v) = std::env::var("WATCH") {
            cfg.watch = parse_bool(&v).context("parse WATCH")?;
        }
        if let Ok(v) = std::env::var("INJECT_FALLBACK_COPY") {
            cfg.inject_fallback_copy = parse_bool(&v).context("parse INJECT_FALLBACK_COPY")?;
        }
        if let Ok(v) = std::env::var("SECRETS_PROVIDER") {
            cfg.provider = v;
        }
        Ok(cfg)
    }
}

fn parse_bool(s: &str) -> anyhow::Result<bool> {
    match s.to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "y" | "on" => Ok(true),
        "0" | "false" | "no" | "n" | "off" => Ok(false),
        other => anyhow::bail!("invalid boolean: {}", other),
    }
}
