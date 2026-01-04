use crate::cmd::Cli;
use crate::compose::MetadataError;
use crate::error::LocketError;
use clap::{Arg, Command, CommandFactory};
use serde::Serialize;

#[derive(Serialize)]
#[serde(rename_all = "lowercase")]
enum ParamType {
    String,
    Boolean,
}

#[derive(Serialize)]
struct ProviderMetadata {
    description: String,
    up: CommandMetadata,
    down: CommandMetadata,
}

#[derive(Serialize)]
struct CommandMetadata {
    parameters: Vec<Parameter>,
}

#[derive(Serialize)]
struct Parameter {
    name: String,
    description: String,
    required: bool,
    #[serde(rename = "type")]
    param_type: ParamType,
    #[serde(skip_serializing_if = "Option::is_none")]
    default: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "enum")]
    enum_values: Option<String>,
}

pub async fn metadata(_project: String) -> Result<(), LocketError> {
    let cmd = Cli::command();
    let compose = get_subcommand(&cmd, "compose")?;

    let meta = ProviderMetadata {
        description: cmd.get_about().unwrap_or_default().to_string(),
        up: extract_metadata(get_subcommand(compose, "up")?),
        down: extract_metadata(get_subcommand(compose, "down")?),
    };

    serde_json::to_writer_pretty(std::io::stdout(), &meta).map_err(MetadataError::from)?;

    Ok(())
}

fn get_subcommand<'a>(cmd: &'a Command, name: &str) -> Result<&'a Command, MetadataError> {
    cmd.get_subcommands()
        .find(|s| s.get_name() == name)
        .ok_or_else(|| MetadataError::MissingSubcommand(name.to_string()))
}

fn extract_metadata(cmd: &Command) -> CommandMetadata {
    let parameters = cmd
        .get_arguments()
        .filter_map(Parameter::from_arg)
        .collect();

    CommandMetadata { parameters }
}

impl Parameter {
    fn from_arg(arg: &Arg) -> Option<Self> {
        // Skip positional args, help, version, and hidden args
        if arg.is_positional()
            || arg.get_id() == "help"
            || arg.get_id() == "version"
            || arg.is_hide_set()
        {
            return None;
        }

        let name = arg.get_long()?.to_string();

        let param_type = if arg.get_action().takes_values() {
            ParamType::String
        } else {
            ParamType::Boolean
        };

        let description = arg.get_help().map(|s| s.to_string()).unwrap_or_default();

        let default = arg
            .get_default_values()
            .first()
            .map(|v| v.to_string_lossy().to_string());

        let enum_values = if !arg.get_possible_values().is_empty() {
            Some(
                arg.get_possible_values()
                    .iter()
                    .map(|v| v.get_name())
                    .collect::<Vec<_>>()
                    .join(","),
            )
        } else {
            None
        };

        Some(Parameter {
            name,
            description,
            required: arg.is_required_set(),
            param_type,
            default,
            enum_values,
        })
    }
}
