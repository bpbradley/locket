use clap::{Arg, Args, Command, CommandFactory};
use indexmap::IndexMap;
use locket::cmd::Cli;
use locket::cmd::{ExecArgs, InjectArgs};
use locket::config::{ApplyDefaults, LocketDocDefaults};
use locket::volume::config::VolumeArgs;
use serde::Serialize;
use std::collections::HashMap;
use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;

#[derive(Args)]
pub struct DocGenerator {
    /// Check if docs are up to date instead of generating
    #[arg(long, default_value_t = false)]
    check: bool,
    /// Output directory for generated docs, relative to project root
    #[arg(long, env = "DOCS_DIR", default_value = "docs")]
    dir: PathBuf,
    /// Output filename for generated docs, relative to project root
    #[arg(long, env = "DOCS_FILE", default_value = "CONFIGURATION.md")]
    file: PathBuf,
}

impl Default for DocGenerator {
    fn default() -> Self {
        Self {
            check: false,
            dir: PathBuf::from("docs"),
            file: PathBuf::from("CONFIGURATION.md"),
        }
    }
}

impl DocGenerator {
    pub fn new(check: bool, dir: PathBuf, file: PathBuf) -> Self {
        Self { check, dir, file }
    }
    pub fn generate(self) -> anyhow::Result<()> {
        let mut cmd = Cli::command();

        clean_environment(&cmd);

        if let Some(sub) = cmd.find_subcommand_mut("inject") {
            let defaults = InjectArgs::get_defaults();
            patch_defaults(sub, &defaults);
        }

        if let Some(sub) = cmd.find_subcommand_mut("exec") {
            let defaults = ExecArgs::get_defaults();
            patch_defaults(sub, &defaults);
        }

        if let Some(sub) = cmd.find_subcommand_mut("volume") {
            let defaults = VolumeArgs::get_defaults();
            patch_defaults(sub, &defaults);
        }

        let app_name = cmd.get_name().to_string();
        let version = cmd.get_version().unwrap_or("0.0.0");

        let docs_dir = &self.dir;
        if !docs_dir.exists() {
            if self.check {
                anyhow::bail!("Documentation directory '{}' missing", docs_dir.display());
            }
            fs::create_dir_all(docs_dir)?;
        }

        let mut index_buffer = Vec::new();
        writeln!(
            &mut index_buffer,
            "# {} {} -- Configuration Reference",
            app_name, version
        )?;
        writeln!(&mut index_buffer, "## Commands\n")?;

        for sub in cmd.get_subcommands() {
            if sub.is_hide_set() {
                continue;
            }
            let name = sub.get_name();
            let filename = format!("{}.md", name);

            if let Some(about) = sub.get_about() {
                writeln!(
                    &mut index_buffer,
                    "- [`{}`](./{}) - {}",
                    name, filename, about
                )?;
            } else {
                writeln!(&mut index_buffer, "- [`{}`](./{})", name, filename)?;
            }
        }
        write_or_verify(&docs_dir.join(&self.file), &index_buffer, self.check)?;

        for sub in cmd.get_subcommands() {
            if sub.is_hide_set() {
                continue;
            }
            let name = sub.get_name();
            let filename = format!("{}.md", name);
            let sub_path = docs_dir.join(&filename);

            let mut sub_buffer = Vec::new();

            writeln!(
                &mut sub_buffer,
                "[Return to Index](./{})\n",
                self.file.display()
            )?;

            writeln!(sub_buffer, "> [!TIP]")?;
            writeln!(
                sub_buffer,
                "> All configuration options can be set via command line arguments OR \
            environment variables. CLI arguments take precedence.\n"
            )?;

            write_command_section(&mut sub_buffer, sub, &app_name)?;

            if name == "inject" {
                write_toml_section::<InjectArgs>(&mut sub_buffer, sub)?;
            } else if name == "exec" {
                write_toml_section::<ExecArgs>(&mut sub_buffer, sub)?;
            } else if name == "volume" {
                write_toml_section::<VolumeArgs>(&mut sub_buffer, sub)?;
            }

            if has_visible_subcommands(sub) {
                for child in sub.get_subcommands() {
                    if !child.is_hide_set() {
                        writeln!(&mut sub_buffer, "\n---\n")?;

                        let parent_context = format!("{} {}", app_name, name);
                        write_command_section(&mut sub_buffer, child, &parent_context)?;
                    }
                }
            }

            write_or_verify(&sub_path, &sub_buffer, self.check)?;
        }

        if self.check {
            println!("Documentation is up to date.");
        } else {
            println!("Documentation generated in {}", docs_dir.display());
        }

        Ok(())
    }
}

fn write_toml_section<T>(writer: &mut impl Write, cmd: &Command) -> io::Result<()>
where
    T: Default + ApplyDefaults + Serialize + locket::config::ConfigStructure,
{
    let config = T::default().apply_defaults();

    let value =
        toml::Value::try_from(&config).map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
    let active_map = value
        .as_table()
        .ok_or_else(|| io::Error::new(io::ErrorKind::Other, "Config is not a table"))?;

    // iterates fields in definition order
    let all_keys = T::get_structure();

    let mut simple_keys = Vec::new();
    let mut table_keys = Vec::new();

    for (key, docs) in all_keys {
        if let Some(val) = active_map.get(&key) {
            if is_toml_table_section(val) {
                table_keys.push((key, docs));
            } else {
                simple_keys.push((key, docs));
            }
        } else {
            // Missing keys (None) are treated as simple comments
            simple_keys.push((key, docs));
        }
    }

    writeln!(writer, "\n## TOML Reference\n")?;
    writeln!(writer, "> [!TIP]")?;
    writeln!(
        writer,
        "> Settings can be provided via config.toml as well, using the --config option."
    )?;
    writeln!(
        writer,
        "> Provided is the reference configuration in TOML format\n"
    )?;
    writeln!(writer, "```toml")?;

    // Helper to write keys
    let mut write_keys = |keys: Vec<(String, Option<String>)>| -> io::Result<()> {
        for (key, docs) in keys {
            // Try to find the corresponding argument help in the Clap Command
            if let Some(arg) = cmd.get_arguments().find(|a| a.get_long() == Some(&key)) {
                if let Some(help) = arg.get_help() {
                    writeln!(writer, "# {}", help)?;
                }
            }

            // Inject docsif present
            if let Some(doc) = docs {
                // Split each line, trim, and add comment prefix
                for line in doc.lines() {
                    writeln!(writer, "# {}", line.trim())?;
                }
            }

            if let Some(val) = active_map.get(&key) {
                let mut mini_map = toml::map::Map::new();
                mini_map.insert(key.clone(), val.clone());
                let line = toml::to_string(&mini_map).unwrap();
                write!(writer, "{}", line)?;
            } else {
                writeln!(writer, "# {} = ...", key)?;
            }
            writeln!(writer)?;
        }
        Ok(())
    };

    write_keys(simple_keys)?;
    write_keys(table_keys)?;

    writeln!(writer, "```")?;

    Ok(())
}

/// Determines if a TOML will be rendered as section or inline value
fn is_toml_table_section(val: &toml::Value) -> bool {
    match val {
        toml::Value::Table(_) => true,
        toml::Value::Array(arr) => {
            if let Some(first) = arr.first() {
                first.is_table()
            } else {
                false
            }
        }
        _ => false,
    }
}

fn clean_environment(cmd: &Command) {
    for arg in cmd.get_arguments() {
        if let Some(env_os) = arg.get_env() {
            // Unset this variable for the current process
            // so Clap doesn't think it's active.
            unsafe { std::env::remove_var(env_os) };
        }
    }

    // Recurse into subcommands (inject, exec, etc.)
    for sub in cmd.get_subcommands() {
        clean_environment(sub);
    }
}

fn patch_defaults(cmd: &mut Command, defaults: &HashMap<String, String>) {
    let mut updates = Vec::new();

    for arg in cmd.get_arguments() {
        if let Some(long) = arg.get_long() {
            if let Some(def) = defaults.get(long) {
                updates.push((arg.get_id().clone(), def.clone()));
            }
        }
    }
    let mut owned_cmd = std::mem::take(cmd);

    for (id, val) in updates {
        owned_cmd = owned_cmd.mut_arg(id, |arg| arg.default_value(val));
    }
    *cmd = owned_cmd;
}

fn has_visible_subcommands(cmd: &Command) -> bool {
    cmd.get_subcommands().any(|s| !s.is_hide_set())
}

fn write_or_verify(path: &PathBuf, content: &[u8], check: bool) -> anyhow::Result<()> {
    let content_str = std::str::from_utf8(content)?;

    if check {
        if !path.exists() {
            anyhow::bail!("File missing: {}", path.display());
        }
        let current = fs::read_to_string(path)?;
        if current.replace("\r\n", "\n") != content_str.replace("\r\n", "\n") {
            anyhow::bail!(
                "Documentation is stale for: {}. Run 'cargo xtask docs' to update.",
                path.display()
            );
        }
    } else {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, content)?;
        println!("Generated {}", path.display());
    }
    Ok(())
}

fn write_command_section(
    writer: &mut impl Write,
    cmd: &Command,
    parent_name: &str,
) -> io::Result<()> {
    let cmd_name = cmd.get_name();

    let display_name = format!("{} {}", parent_name, cmd_name);

    writeln!(writer, "## `{}`\n", display_name)?;

    if let Some(about) = cmd.get_long_about().or_else(|| cmd.get_about()) {
        writeln!(writer, "{}\n", about)?;
    }
    let mut groups: IndexMap<Option<String>, Vec<&Arg>> = IndexMap::new();
    for arg in cmd.get_arguments() {
        if arg.get_id() == "help" || arg.get_id() == "version" || arg.is_hide_set() {
            continue;
        }
        let heading = arg.get_help_heading().map(|s| s.to_string());
        groups.entry(heading).or_default().push(arg);
    }

    if groups.is_empty() {
        writeln!(writer, "_No options._\n")?;
        return Ok(());
    }

    for (heading, args) in groups {
        let title = heading.as_deref().unwrap_or("Options");
        writeln!(writer, "### {}\n", title)?;
        writeln!(writer, "| Command | Env | Default | Description |")?;
        writeln!(writer, "| :--- | :--- | :--- | :--- |")?;

        for arg in args {
            write_arg_row(writer, arg)?;
        }
    }

    Ok(())
}

fn write_arg_row(writer: &mut impl Write, arg: &Arg) -> io::Result<()> {
    let flag = if let Some(l) = arg.get_long() {
        format!("`--{}`", l)
    } else {
        format!("`<{}>`", arg.get_id())
    };

    let env = arg
        .get_env()
        .map(|e| format!("`{}`", e.to_string_lossy()))
        .unwrap_or_else(|| "".to_string());

    let default = if !arg.get_default_values().is_empty() {
        let vals: Vec<_> = arg
            .get_default_values()
            .iter()
            .map(|v| v.to_string_lossy())
            .collect();
        format!("`{}`", vals.join(", "))
    } else {
        "".to_string()
    };

    let help_msg = arg
        .get_long_help()
        .or_else(|| arg.get_help()) // Fallback to short help if no long help exists
        .map(|s| s.to_string())
        .unwrap_or_default();
    let mut help = help_msg.replace("{n}", "<br>").replace("\n", "<br>"); // Preserve line breaks in markdown tables

    let possible_values = arg.get_possible_values();
    if !possible_values.is_empty() {
        let values_list: Vec<String> = possible_values
            .iter()
            .map(|v| {
                let name = v.get_name();
                match v.get_help() {
                    Some(h) => {
                        format!("- `{}`: {}", name, h)
                    }
                    None => format!("- `{}`", name),
                }
            })
            .collect();
        help = format!(
            "{} <br><br> **Choices:**<br>{}",
            help,
            values_list.join("<br>")
        );
    }

    writeln!(writer, "| {} | {} | {} | {} |", flag, env, default, help)?;

    Ok(())
}
