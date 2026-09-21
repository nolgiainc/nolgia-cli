use clap::{Command, CommandFactory, FromArgMatches};

use crate::Cli;

pub fn parse() -> Cli {
    let mut command = Cli::command();
    command.build();
    // Clap replaces a global Append vector with the deepest supplied vector.
    // Keep inherited definitions, but collect this one flag at each parse level.
    let matches = local_fields(command).get_matches();
    let mut fields = Vec::new();
    let mut level = Some(&matches);
    while let Some(args) = level {
        if let Some(values) = args.get_many::<String>("field") {
            fields.extend(values.cloned());
        }
        level = args.subcommand().map(|(_, child)| child);
    }
    let mut cli = Cli::from_arg_matches(&matches).unwrap_or_else(|error| error.exit());
    cli.field = fields;
    cli
}

fn local_fields(mut command: Command) -> Command {
    for child in command.get_subcommands_mut() {
        *child = local_fields(std::mem::take(child));
    }
    command.mut_args(|arg| {
        if arg.get_id() == "field" {
            arg.global(false)
        } else {
            arg
        }
    })
}
