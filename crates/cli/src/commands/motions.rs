//! The camera-move library for video generation (NOL-864): a static,
//! model-agnostic catalog of named camera moves the API serves on
//! `GET /motions`. A `gen video --motion <id> [--motion-strength subtle|medium|strong]`
//! request names one and the server appends the move's prompt fragment to
//! the prompt (never replacing it). Public, identical for every caller.

use anyhow::Result;
use clap::Subcommand;

use super::CommandContext;
use crate::output::{OutputFormat, print_json};

#[derive(Subcommand, Debug)]
pub enum MotionsCommand {
    /// List the camera moves (id, name, what it does) for `gen video --motion`
    List,
}

pub async fn run(command: MotionsCommand, ctx: &CommandContext) -> Result<()> {
    match command {
        MotionsCommand::List => list(ctx).await,
    }
}

async fn list(ctx: &CommandContext) -> Result<()> {
    let catalog = match ctx.client().list_motions().send().await {
        Ok(response) => response.into_inner(),
        Err(err) => return Err(super::api_error(err, "listing camera moves").await),
    };
    match ctx.format() {
        OutputFormat::Json => print_json(&catalog),
        OutputFormat::Text => {
            let id_width = catalog
                .motions
                .iter()
                .map(|m| m.id.len())
                .max()
                .unwrap_or(0);
            let name_width = catalog
                .motions
                .iter()
                .map(|m| m.name.len())
                .max()
                .unwrap_or(0);
            for motion in &catalog.motions {
                println!(
                    "{:id_width$}  {:name_width$}  {}",
                    motion.id, motion.name, motion.description,
                );
            }
            println!(
                "\n`nolgia gen video --motion <id> [--motion-strength subtle|medium|strong]` appends the move to your prompt (default strength: medium). `--json` shows each strength's exact prompt fragment."
            );
            Ok(())
        }
    }
}
