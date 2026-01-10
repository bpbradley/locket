use clap::CommandFactory;
use locket::cmd::{Cli, ExecArgs, InjectArgs};
use serde::Serialize;
use std::collections::HashSet;
use std::fmt::Debug;

#[test]
fn test_inject_parity() {
    check_parity::<InjectArgs>(&Cli::command(), "inject");
}

#[test]
fn test_exec_parity() {
    check_parity::<ExecArgs>(&Cli::command(), "exec");
}

fn check_parity<T>(root: &clap::Command, subcommand_name: &str)
where
    T: Serialize + Default + Debug,
{
    let subcommand = root
        .get_subcommands()
        .find(|s| s.get_name() == subcommand_name)
        .expect("Subcommand not found");

    if !subcommand
        .get_arguments()
        .any(|a| a.get_long() == Some("config"))
    {
        panic!("Subcommand '{}' missing --config flag.", subcommand_name);
    }

    let instance = T::default();
    let json = serde_json::to_value(&instance).expect("Serialization failed");
    let mut config_keys = HashSet::new();
    collect_keys_recursive(&json, &mut config_keys);

    let mut errors = Vec::new();

    for key in config_keys {
        if matches!(key.as_str(), "help" | "version") {
            continue;
        }

        // check for flag
        if find_flag(subcommand, &key) {
            continue;
        }

        // check if positional match
        if find_positional(subcommand, &key) {
            continue;
        }

        errors.push(format!("Key '{}' missing from CLI", key));
    }

    if !errors.is_empty() {
        panic!("Parity Mismatch for '{}':\n{:#?}", subcommand_name, errors);
    }
}

fn find_flag(cmd: &clap::Command, name: &str) -> bool {
    cmd.get_arguments().any(|a| a.get_long() == Some(name))
}

fn find_positional(cmd: &clap::Command, name: &str) -> bool {
    cmd.get_arguments().any(|a| {
        let is_positional = a.get_long().is_none();

        // Clap IDs are snake_case, Config keys are kebab-case.
        let id_matches = a.get_id().as_str().replace('_', "-") == name;

        is_positional && id_matches
    })
}

fn collect_keys_recursive(value: &serde_json::Value, keys: &mut HashSet<String>) {
    if let Some(map) = value.as_object() {
        for (k, v) in map {
            keys.insert(k.clone());
            if v.is_object() {
                collect_keys_recursive(v, keys);
            }
        }
    }
}
