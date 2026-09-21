use anyhow::{Context, Result};
use clap::Subcommand;
use serde::Serialize;

use crate::output::{OutputFormat, print_json};

use super::CommandContext;

#[derive(Subcommand, Debug)]
pub enum AccountCommand {
    Me,
    Usage,
}

#[derive(Serialize)]
struct UsageSummary {
    jobs_visible: usize,
    assets_visible: usize,
}

pub async fn run(command: AccountCommand, ctx: &CommandContext) -> Result<()> {
    match command {
        AccountCommand::Me => me(ctx).await,
        AccountCommand::Usage => usage(ctx).await,
    }
}

async fn me(ctx: &CommandContext) -> Result<()> {
    let user = ctx
        .client()
        .get_current_user()
        .send()
        .await
        .context("fetching current user")?
        .into_inner();
    match ctx.format() {
        OutputFormat::Json => print_json(ctx.output(), &user),
        OutputFormat::Text => {
            println!("{} {}", user.id, user.email);
            Ok(())
        }
    }
}

async fn usage(ctx: &CommandContext) -> Result<()> {
    let jobs = ctx
        .client()
        .list_jobs()
        .send()
        .await
        .context("listing jobs for usage")?
        .into_inner();
    let assets = ctx
        .client()
        .list_assets()
        .send()
        .await
        .context("listing assets for usage")?
        .into_inner();
    let summary = UsageSummary {
        jobs_visible: jobs.items.len(),
        assets_visible: assets.items.len(),
    };
    match ctx.format() {
        OutputFormat::Json => print_json(ctx.output(), &summary),
        OutputFormat::Text => {
            println!(
                "jobs: {} assets: {}",
                summary.jobs_visible, summary.assets_visible
            );
            Ok(())
        }
    }
}
