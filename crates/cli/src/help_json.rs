use std::io::Write;

use anyhow::Result;
use clap::{Arg, ArgAction, Command};
use serde::Serialize;

#[derive(Serialize)]
struct CommandHelp {
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    version: Option<String>,
    about: Option<String>,
    long_about: Option<String>,
    aliases: Vec<String>,
    args: Vec<ArgumentHelp>,
    subcommands: Vec<CommandHelp>,
}

#[derive(Serialize)]
struct ArgumentHelp {
    id: String,
    long: Option<String>,
    short: Option<char>,
    value_name: Option<String>,
    help: Option<String>,
    long_help: Option<String>,
    takes_value: bool,
    multiple: bool,
    required: bool,
    global: bool,
    env: Option<String>,
    default: Option<String>,
    possible_values: Vec<PossibleValueHelp>,
}

#[derive(Serialize)]
struct PossibleValueHelp {
    name: String,
    help: Option<String>,
}

pub fn print(mut command: Command) -> Result<()> {
    // Building the entire tree propagates global arguments to every leaf.
    command.build();
    let mut stdout = std::io::stdout().lock();
    serde_json::to_writer_pretty(&mut stdout, &describe(&command, true))?;
    writeln!(stdout)?;
    Ok(())
}

fn describe(command: &Command, root: bool) -> CommandHelp {
    CommandHelp {
        name: command.get_name().to_owned(),
        version: root.then(|| env!("CARGO_PKG_VERSION").to_owned()),
        about: command.get_about().map(ToString::to_string),
        long_about: command.get_long_about().map(ToString::to_string),
        aliases: command.get_visible_aliases().map(str::to_owned).collect(),
        args: command
            .get_arguments()
            .filter(|arg| {
                !arg.is_hide_set()
                    && !matches!(
                        arg.get_action(),
                        ArgAction::Help
                            | ArgAction::HelpShort
                            | ArgAction::HelpLong
                            | ArgAction::Version
                    )
            })
            .map(describe_arg)
            .collect(),
        subcommands: command
            .get_subcommands()
            .filter(|child| !child.is_hide_set() && child.get_name() != "help")
            .map(|child| describe(child, false))
            .collect(),
    }
}

fn describe_arg(arg: &Arg) -> ArgumentHelp {
    let takes_value = arg.get_action().takes_values();
    ArgumentHelp {
        id: arg.get_id().to_string(),
        long: arg.get_long().map(str::to_owned),
        short: arg.get_short(),
        value_name: if takes_value {
            arg.get_value_names()
                .map(|names| {
                    names
                        .iter()
                        .map(|name| name.as_str())
                        .collect::<Vec<_>>()
                        .join(" ")
                })
                .or_else(|| arg.is_positional().then(|| arg.get_id().to_string()))
        } else {
            None
        },
        help: arg.get_help().map(ToString::to_string),
        long_help: arg.get_long_help().map(ToString::to_string),
        takes_value,
        multiple: matches!(arg.get_action(), ArgAction::Append | ArgAction::Count)
            || arg
                .get_num_args()
                .is_some_and(|range| range.max_values() > 1),
        required: arg.is_required_set(),
        global: arg.is_global_set(),
        // Clap stores the resolved environment separately. Export only its name
        // and the declared default, never the resolved value (NOL-317).
        env: arg
            .get_env()
            .map(|name| name.to_string_lossy().into_owned()),
        default: if takes_value {
            arg.get_default_values()
                .first()
                .map(|value| value.to_string_lossy().into_owned())
        } else {
            None
        },
        possible_values: arg
            .get_possible_values()
            .into_iter()
            .filter(|value| !value.is_hide_set())
            .map(|value| PossibleValueHelp {
                name: value.get_name().to_owned(),
                help: value.get_help().map(ToString::to_string),
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_omits_hidden_surface_and_inherits_global_arguments() {
        let mut command = Command::new("root")
            .version("1")
            .arg(
                Arg::new("json")
                    .long("json")
                    .global(true)
                    .action(ArgAction::SetTrue),
            )
            .arg(Arg::new("secret").long("secret").hide(true))
            .subcommand(Command::new("hidden").hide(true))
            .subcommand(
                Command::new("child")
                    .visible_alias("shown")
                    .alias("unlisted")
                    .arg(Arg::new("path").value_name("PATH")),
            );
        command.build();
        let metadata = describe(&command, true);
        let value = serde_json::to_value(metadata).expect("serialized help");
        assert_eq!(value["args"].as_array().expect("args").len(), 1);
        let children = value["subcommands"].as_array().expect("subcommands");
        assert_eq!(children.len(), 1);
        assert!(children[0].get("version").is_none());
        assert_eq!(children[0]["aliases"], serde_json::json!(["shown"]));
        let args = children[0]["args"].as_array().expect("child args");
        assert!(
            args.iter()
                .any(|arg| arg["id"] == "json" && arg["global"] == true)
        );
        let positional = args
            .iter()
            .find(|arg| arg["id"] == "path")
            .expect("positional");
        assert_eq!(positional["long"], serde_json::Value::Null);
        assert_eq!(positional["value_name"], "PATH");
    }
}
