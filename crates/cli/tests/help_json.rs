use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::Value;

fn cmd() -> Command {
    let mut command = Command::cargo_bin("nolgia").expect("nolgia binary");
    command.env_remove("NOLGIA_TOKEN");
    command.env("NOLGIA_NO_UPDATE_CHECK", "1");
    command
}

fn command_named<'a>(tree: &'a Value, name: &str) -> &'a Value {
    tree["subcommands"]
        .as_array()
        .expect("subcommands array")
        .iter()
        .find(|command| command["name"] == name)
        .unwrap_or_else(|| panic!("missing subcommand {name}"))
}

fn arg_named<'a>(tree: &'a Value, id: &str) -> &'a Value {
    tree["args"]
        .as_array()
        .expect("args array")
        .iter()
        .find(|arg| arg["id"] == id)
        .unwrap_or_else(|| panic!("missing argument {id} for {}", tree["name"]))
}

#[test]
fn help_json_describes_commands_without_resolving_auth() {
    let output = cmd()
        .args(["--help-json", "--api-url", "not-a-server-url"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let tree: Value = serde_json::from_slice(&output).expect("help JSON");
    assert_eq!(tree["name"], "nolgia");
    assert_eq!(tree["version"], env!("CARGO_PKG_VERSION"));
    let api = command_named(&tree, "api");
    for id in ["method", "path", "body"] {
        assert_eq!(arg_named(api, id)["id"], id);
    }
    assert_eq!(arg_named(api, "method")["long"], Value::Null);
    assert!(arg_named(api, "method")["value_name"].is_string());
    assert_eq!(arg_named(api, "body")["takes_value"], true);
    assert_eq!(
        command_named(command_named(&tree, "jobs"), "get")["name"],
        "get"
    );
    assert_eq!(
        command_named(&tree, "org")["aliases"],
        serde_json::json!(["workspace"])
    );
    assert_eq!(arg_named(&tree, "json")["takes_value"], false);
    assert_eq!(arg_named(&tree, "json")["value_name"], Value::Null);
    assert_eq!(arg_named(&tree, "json")["default"], Value::Null);
    assert_eq!(arg_named(&tree, "field")["multiple"], true);
    assert_eq!(arg_named(&tree, "output")["value_name"], "FORMAT");
    assert_eq!(arg_named(&tree, "token")["env"], "NOLGIA_TOKEN");
    assert_eq!(arg_named(&tree, "token")["default"], Value::Null);
    assert_eq!(
        arg_named(&tree, "api_url")["default"],
        "https://api.nolgia.ai"
    );
    assert!(
        String::from_utf8(output)
            .expect("UTF-8 JSON")
            .starts_with("{\n  \"name\": \"nolgia\",\n  \"version\": ")
    );
}

#[test]
fn help_json_never_exposes_environment_values() {
    let output = cmd()
        .env("NOLGIA_TOKEN", "nol_leak_canary_value")
        .env("NOLGIA_API_URL", "https://leak_canary.invalid")
        .env("NOLGIA_IDEMPOTENCY_KEY", "leak_canary_key")
        .arg("--help-json")
        .assert()
        .success()
        .stdout(predicate::str::contains("leak_canary").not())
        .get_output()
        .stdout
        .clone();
    let tree: Value = serde_json::from_slice(&output).expect("help JSON");
    assert_eq!(
        arg_named(&tree, "api_url")["default"],
        "https://api.nolgia.ai"
    );
}

#[test]
fn documented_command_tree_preserves_global_output_flags_and_readme_index() {
    let output = cmd()
        .arg("--help-json")
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let tree: Value = serde_json::from_slice(&output).expect("help JSON");
    check_tree(&tree, true);
    let readme = include_str!("../../../README.md");
    let index = readme
        .split_once("## Command index\n")
        .expect("command index section")
        .1
        .split("\n## ")
        .next()
        .expect("command index content");
    let mut entries = 0;
    for row in index.lines().filter(|line| line.starts_with("| `")) {
        let column = row.split('|').nth(1).expect("first table column");
        for entry in column.split(',') {
            let name = entry
                .trim()
                .trim_start_matches('`')
                .split(['`', ' ', '('])
                .next()
                .expect("top-level command name");
            assert_eq!(command_named(&tree, name)["name"], name);
            entries += 1;
        }
    }
    assert!(entries > 0, "README command index must contain commands");
}

fn check_tree(command: &Value, root: bool) {
    assert_eq!(command.get("version").is_some(), root);
    let args = command["args"].as_array().expect("args array");
    assert!(
        args.iter()
            .all(|arg| arg["id"] != "help" && arg["id"] != "version")
    );
    let children = command["subcommands"]
        .as_array()
        .expect("subcommands array");
    if children.is_empty() {
        for flag in ["json", "field", "output"] {
            assert_eq!(arg_named(command, flag)["global"], true);
        }
    }
    for child in children {
        assert_ne!(child["name"], "help");
        check_tree(child, false);
    }
}

#[test]
fn bare_invocation_retains_usage_error() {
    cmd()
        .assert()
        .code(2)
        .stdout("")
        .stderr(predicate::str::contains("Usage:"));
}

#[test]
fn json_without_command_retains_missing_subcommand_error() {
    cmd()
        .arg("--json")
        .assert()
        .code(2)
        .stdout("")
        .stderr(predicate::str::contains("subcommand"));
}
