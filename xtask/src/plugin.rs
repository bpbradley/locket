use clap::{Args, Command, CommandFactory};
use locket::cmd::Cli;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use serde_json::ser::{PrettyFormatter, Serializer as JsonSerializer};
use std::path::PathBuf;
use std::{fs, process};

#[derive(Args)]
pub struct PluginConfigArgs {
    /// Path to config.json
    #[arg(long, default_value = "docker/plugin/config.json")]
    config_path: PathBuf,

    /// Check if config is up to date instead of generating
    #[arg(long)]
    check: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct DockerEnv {
    name: String,
    description: String,
    settable: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    value: Option<String>,
}

impl PluginConfigArgs {
    pub fn generate(self) -> anyhow::Result<()> {
        if !self.config_path.exists() {
            anyhow::bail!("Config file not found at: {}", self.config_path.display());
        }
        let content = fs::read_to_string(&self.config_path)?;
        let mut root: Value = serde_json::from_str(&content)?;

        let cmd = Cli::command();
        let volume_cmd = find_subcommand(&cmd, "volume").expect("volume subcommand must exist");

        let mut new_envs = Vec::new();

        for arg in volume_cmd.get_arguments() {
            if let Some(env_os) = arg.get_env() {
                let name = env_os.to_string_lossy().to_string();

                let description = arg
                    .get_help()
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| "No description".to_string());

                let value = if !arg.get_default_values().is_empty() {
                    arg.get_default_values()
                        .first()
                        .map(|v| v.to_string_lossy().to_string())
                } else {
                    None
                };

                new_envs.push(DockerEnv {
                    name,
                    description,
                    settable: vec!["value".to_string()],
                    value,
                });
            }
        }

        new_envs.sort_by(|a, b| a.name.cmp(&b.name));

        let env_json = serde_json::to_value(&new_envs)?;
        root["env"] = env_json;

        let mut new_content_vec = Vec::new();
        let formatter = PrettyFormatter::with_indent(b"    ");
        let mut ser = JsonSerializer::with_formatter(&mut new_content_vec, formatter);
        root.serialize(&mut ser)?;

        new_content_vec.push(b'\n');

        let new_content = String::from_utf8(new_content_vec)?;

        if self.check {
            if content.trim().replace("\r\n", "\n") != new_content.trim().replace("\r\n", "\n") {
                eprintln!("Plugin configuration is stale. Run 'cargo xtask plugin' to update.");
                process::exit(1);
            }
            println!("Plugin configuration is up to date.");
        } else {
            fs::write(&self.config_path, new_content)?;
            println!(
                "Updated plugin configuration at {}",
                self.config_path.display()
            );
        }

        Ok(())
    }
}

fn find_subcommand<'a>(cmd: &'a Command, name: &str) -> Option<&'a Command> {
    cmd.get_subcommands().find(|s| s.get_name() == name)
}
