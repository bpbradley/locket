use clap::{Arg, Args, Command, CommandFactory, Parser, Subcommand};
use indexmap::IndexMap;
use locket::cmd::Cli;
use std::fs::{self, File};
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
        fs::create_dir(docs_dir)?;
    }

    let mut index_file = File::create(docs_dir.join(&config.file))?;

    writeln!(index_file, "# {} v{} Configuration Refereence", app_name, version)?;
    writeln!(index_file, "## Commands\n")?;

    for sub in cmd.get_subcommands() {
        if sub.is_hide_set() {
            continue;
        }

        let name = sub.get_name();
        let filename = format!("{}.md", name);

        // Link in the index file
        if let Some(about) = sub.get_about() {
            writeln!(index_file, "- [`{}`](./{}) — {}", name, filename, about)?;
        } else {
            writeln!(index_file, "- [`{}`](./{})", name, filename)?;
        }

        // Command files
        let sub_path = docs_dir.join(&filename);
        let mut sub_file = File::create(&sub_path)?;
        write_subcommand_docs(&mut sub_file, &config, sub, &app_name)?;

        println!("Generated {}", sub_path.display());
    }

    println!("Generated {}/{}", docs_dir.display(), config.file.display());
    Ok(())
}

fn write_subcommand_docs(
    file: &mut File,
    config: &DocGenerator,
    cmd: &Command,
    app_name: &str,
) -> io::Result<()> {
    let cmd_name = cmd.get_name();

    writeln!(file, "[Return to Index](./{})\n", config.file.display())?;

    writeln!(file, "# {} {}\n", app_name, cmd_name)?;
    writeln!(file, "> [!TIP]")?;
    writeln!(
        file,
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
        writeln!(file, "_No configuration options._\n")?;
        return Ok(());
    }

    // Render Tables
    for (heading, args) in groups {
        let title = heading.as_deref().unwrap_or("General");

        writeln!(file, "### {}\n", title)?;
        writeln!(file, "| Command | Env | Default | Description |")?;
        writeln!(file, "| :--- | :--- | :--- | :--- |")?;

        for arg in args {
            write_arg_row(file, arg)?;
        }
    }

    Ok(())
}

fn write_arg_row(file: &mut File, arg: &Arg) -> io::Result<()> {
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

    writeln!(file, "| {} | {} | {} | {} |", flag, env, default, help)?;

    Ok(())
}
