use crate::cmd::Cli;
use clap::{Command, CommandFactory};
use serde::Serialize;

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
    param_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    default: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "enum")]
    enum_values: Option<String>,
}

pub async fn metadata(_project: String) -> sysexits::ExitCode {
    let cmd = Cli::command();

    let compose_cmd =
        find_subcommand(&cmd, "compose").expect("CLI definition missing 'compose' subcommand");

    let up_meta = build_command_metadata(find_subcommand(compose_cmd, "up"));
    let down_meta = build_command_metadata(find_subcommand(compose_cmd, "down"));

    let meta = ProviderMetadata {
        description: cmd.get_about().unwrap_or_default().to_string(),
        up: up_meta,
        down: down_meta,
    };

    let json = serde_json::to_string_pretty(&meta).unwrap();
    println!("{}", json);

    sysexits::ExitCode::Ok
}

fn find_subcommand<'a>(cmd: &'a Command, name: &str) -> Option<&'a Command> {
    cmd.get_subcommands().find(|s| s.get_name() == name)
}

fn build_command_metadata(cmd: Option<&Command>) -> CommandMetadata {
    let mut parameters = Vec::new();

    if let Some(cmd) = cmd {
        for arg in cmd.get_arguments() {
            if arg.get_long().is_none() || arg.get_id() == "help" || arg.get_id() == "version" {
                continue;
            }

            let name = arg.get_long().unwrap().to_string();
            let description = arg.get_help().map(|s| s.to_string()).unwrap_or_default();
            let required = arg.is_required_set();

            let param_type = if arg.get_action().takes_values() {
                "string".to_string()
            } else {
                "boolean".to_string()
            };

            let default = if !arg.get_default_values().is_empty() {
                Some(arg.get_default_values()[0].to_string_lossy().to_string())
            } else {
                None
            };

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

            parameters.push(Parameter {
                name,
                description,
                required,
                param_type,
                default,
                enum_values,
            });
        }
    }

    CommandMetadata { parameters }
}
