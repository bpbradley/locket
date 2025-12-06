use crate::compose::{ComposeError, ComposeMsg};
use clap::Args;
use sysexits;

#[derive(Args, Debug)]
pub struct UpArgs {
    /// Secrets to be injected as env variables in KEY=VALUE format, separated by commas
    #[arg(long, value_parser = parse_compose_secrets, value_delimiter = ',')]
    secrets: Option<Vec<(String, String)>>,

    /// Service name from Docker Compose
    pub service: String,
}

fn parse_compose_secrets(s: &str) -> Result<(String, String), ComposeError> {
    let pos = s
        .find('=')
        .ok_or_else(|| ComposeError::Argument(format!("Unable to parse secret: {}", s)))?;

    Ok((s[..pos].to_string(), s[pos + 1..].to_string()))
}

pub async fn up(project: String, args: UpArgs) -> sysexits::ExitCode {
    ComposeMsg::debug(format!(
        "[DEBUG]Starting compose up for project: {} with args: {:?}",
        project, args
    ));
    let secrets_map: std::collections::HashMap<String, String> =
        args.secrets.unwrap_or_default().into_iter().collect();

    for (key, value) in secrets_map {
        ComposeMsg::set_env(&key, &value);
        ComposeMsg::debug(format!("Injected secret: {}", key));
    }
    sysexits::ExitCode::Ok
}
