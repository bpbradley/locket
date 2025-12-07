use clap::{Arg, Args, Command, CommandFactory};
use indexmap::IndexMap;
use locket::cmd::Cli;
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
        let cmd = Cli::command();
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

            if has_visible_subcommands(sub) {
                for child in sub.get_subcommands() {
                    if !child.is_hide_set() {
                        writeln!(&mut sub_buffer, "\n---\n")?;

                        let parent_name = format!("{} {}", app_name, name);
                        write_command_section(&mut sub_buffer, child, &parent_name)?;
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

    let display_name = if parent_name == "locket" {
        format!("locket {}", cmd_name)
    } else {
        format!("{} {}", parent_name, cmd_name)
    };

    writeln!(writer, "## `{}`\n", display_name)?;

    if let Some(about) = cmd.get_about() {
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

    let help_msg = arg.get_help().map(|s| s.to_string()).unwrap_or_default();
    let mut help = help_msg.replace("\n", " ");

    let possible_values = arg.get_possible_values();
    if !possible_values.is_empty() {
        let values_list: Vec<_> = possible_values
            .iter()
            .map(|v| format!("`{}`", v.get_name()))
            .collect();
        help = format!("{} <br> **Choices:** {}", help, values_list.join(", "));
    }

    writeln!(writer, "| {} | {} | {} | {} |", flag, env, default, help)?;

    Ok(())
}
