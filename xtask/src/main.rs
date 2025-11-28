use clap::{Arg, Args, Command, CommandFactory, Parser, Subcommand};
use indexmap::IndexMap;
use locket::cmd::Cli;
use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;

#[derive(Parser)]
struct Xtask {
    #[command(subcommand)]
    cmd: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Generate markdown documentation
    Docs(DocGenerator),
}

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

fn main() -> anyhow::Result<()> {
    let args = Xtask::parse();

    match args.cmd {
        Commands::Docs(config) => generate_docs(config),
    }
}

fn generate_docs(config: DocGenerator) -> anyhow::Result<()> {
    let cmd = Cli::command();
    let app_name = cmd.get_name().to_string();
    let version = cmd.get_version().unwrap();

    let docs_dir = &config.dir;
    if !docs_dir.exists() {
        if config.check {
            anyhow::bail!("Documentation directory '{}' missing", docs_dir.display());
        }
        fs::create_dir(docs_dir)?;
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

    write_or_verify(&docs_dir.join(&config.file), &index_buffer, config.check)?;

    for sub in cmd.get_subcommands() {
        if sub.is_hide_set() {
            continue;
        }
        let name = sub.get_name();
        let filename = format!("{}.md", name);
        let sub_path = docs_dir.join(&filename);

        let mut sub_buffer = Vec::new();
        write_subcommand_docs(&mut sub_buffer, &config, sub, &app_name)?;

        write_or_verify(&sub_path, &sub_buffer, config.check)?;
    }

    if config.check {
        println!("Documentation is up to date.");
    } else {
        println!("Documentation generated in {}", docs_dir.display());
    }

    Ok(())
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
        fs::write(path, content)?;
        println!("Generated {}", path.display());
    }
    Ok(())
}

fn write_subcommand_docs(
    writer: &mut impl Write,
    config: &DocGenerator,
    cmd: &Command,
    app_name: &str,
) -> io::Result<()> {
    let cmd_name = cmd.get_name();

    writeln!(writer, "[Return to Index](./{})\n", config.file.display())?;

    writeln!(writer, "# `{} {}`\n", app_name, cmd_name)?;
    writeln!(writer, "> [!TIP]")?;
    writeln!(
        writer,
        "> All configuration options can be set via command line arguments OR \
        environment variables. CLI arguments take precedence.\n"
    )?;

    let mut groups: IndexMap<Option<String>, Vec<&Arg>> = IndexMap::new();
    for arg in cmd.get_arguments() {
        if arg.get_id() == "help" || arg.get_id() == "version" {
            continue;
        }
        let heading = arg.get_help_heading().map(|s| s.to_string());
        groups.entry(heading).or_default().push(arg);
    }

    if groups.is_empty() {
        writeln!(writer, "_No configuration options._\n")?;
        return Ok(());
    }

    // Render Tables
    for (heading, args) in groups {
        let title = heading.as_deref().unwrap_or("General");

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
        .unwrap_or_else(|| "*None*".to_string());

    let default = if !arg.get_default_values().is_empty() {
        let vals: Vec<_> = arg
            .get_default_values()
            .iter()
            .map(|v| v.to_string_lossy())
            .collect();
        format!("`{}`", vals.join(", "))
    } else {
        "*None*".to_string()
    };

    let mut help = arg
        .get_help()
        .map(|h| h.to_string())
        .unwrap_or_default()
        .replace("\n", " ");

    let possible_values = arg.get_possible_values();
    if !possible_values.is_empty() {
        let has_docs = possible_values.iter().any(|v| v.get_help().is_some());

        if has_docs {
            help.push_str("<br><br> **Options:**");
            for v in possible_values {
                let v_name = v.get_name();
                if let Some(h) = v.get_help() {
                    help.push_str(&format!("<br> - `{}`: {}", v_name, h));
                } else {
                    help.push_str(&format!("<br> - `{}`", v_name));
                }
            }
        } else {
            let values_list: Vec<_> = possible_values
                .iter()
                .map(|v| format!("`{}`", v.get_name()))
                .collect();
            help = format!("{} <br><br> **Options:** {}", help, values_list.join(", "));
        }
    }

    writeln!(writer, "| {} | {} | {} | {} |", flag, env, default, help)?;

    Ok(())
}
