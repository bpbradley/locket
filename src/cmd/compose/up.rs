use crate::compose::ComposeMsg;
use crate::provider::Provider;
use crate::secrets::{MemSize, Secret};
use crate::template::Template;
use clap::Args;
use secrecy::ExposeSecret;
use std::borrow::Cow;

#[derive(Args, Debug)]
pub struct UpArgs {
    /// Provider configuration
    #[command(flatten)]
    pub provider: Provider,

    /// Secrets to be injected as environment variables.
    /// Format: KEY=TEMPLATE (e.g. `DB_PASS={{op://vault/item/field}}`)
    /// Supports file indirection: `KEY=@./path/to/file`
    #[arg(
        long,
        env = "LOCKET_SECRETS",
        value_name = "label={{template}} or label=@./path/to/file",
        value_delimiter = ',',
        hide_env_values = true,
        help_heading = None,
    )]
    pub secrets: Vec<Secret>,

    /// Service name from Docker Compose
    #[arg(help_heading = None)]
    pub service: String,
}

pub async fn up(project: String, args: UpArgs) -> sysexits::ExitCode {
    ComposeMsg::info(format!("Starting project: {}", project));

    let provider = match args.provider.build().await {
        Ok(p) => p,
        Err(e) => {
            ComposeMsg::error(format!("Failed to initialize provider: {}", e));
            return sysexits::ExitCode::Config;
        }
    };

    for secret in args.secrets {
        ComposeMsg::info(format!("Processing secret target: {}", secret.key));

        // Apply a reasonable 1MB limit for environment variables
        let raw_template = match secret.source.read().limit(MemSize::from_mb(1)).fetch() {
            Ok(content) => content,
            Err(e) => {
                ComposeMsg::error(format!("Failed to read source for '{}': {}", secret.key, e));
                return sysexits::ExitCode::NoInput;
            }
        };

        let tpl = Template::new(&raw_template);
        let keys = tpl.keys();
        let has_keys = !keys.is_empty();

        let candidates: Vec<&str> = if has_keys {
            keys.into_iter().collect()
        } else {
            vec![raw_template.trim()]
        };

        // Filter for keys this provider actually supports
        let references: Vec<&str> = candidates
            .into_iter()
            .filter(|k| provider.accepts_key(k))
            .collect();

        // If no references match the provider, we pass the raw value through.
        if references.is_empty() {
            ComposeMsg::debug(format!(
                "No resolveable secrets found for '{}'; passing through raw value",
                secret.key
            ));
            ComposeMsg::set_env(&secret.key, &raw_template);
            continue;
        }

        // Fetch Secrets from Provider
        let secret_map = match provider.fetch_map(&references).await {
            Ok(map) => map,
            Err(e) => {
                ComposeMsg::error(format!(
                    "Failed to fetch secrets for '{}': {}",
                    secret.key, e
                ));
                return sysexits::ExitCode::Unavailable;
            }
        };

        // Render Final Value
        let final_value = if has_keys {
            tpl.render_with(|k| secret_map.get(k).map(|s| s.expose_secret()))
        } else {
            match secret_map.get(raw_template.trim()) {
                Some(val) => Cow::Borrowed(val.expose_secret()),
                None => {
                    ComposeMsg::debug(format!(
                        "Provider returned success but value was missing for '{}'",
                        raw_template
                    ));
                    raw_template
                }
            }
        };

        ComposeMsg::set_env(&secret.key, &final_value);
        ComposeMsg::debug(format!("Injected secret: {}", secret.key));
    }

    sysexits::ExitCode::Ok
}
