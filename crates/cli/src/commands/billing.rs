use anyhow::{Context, Result};
use clap::{Args, Subcommand};

use crate::output::{OutputFormat, print_json};

use super::CommandContext;

#[derive(Subcommand, Debug)]
pub enum BillingCommand {
    Subscription,
    Credits,
    Portal(PortalArgs),
}

#[derive(Args, Debug)]
pub struct PortalArgs {
    #[arg(long)]
    pub return_url: Option<String>,
}

pub async fn run(command: BillingCommand, ctx: &CommandContext) -> Result<()> {
    match command {
        BillingCommand::Subscription => subscription(ctx).await,
        BillingCommand::Credits => credits(ctx).await,
        BillingCommand::Portal(args) => portal(args, ctx).await,
    }
}

async fn credits(ctx: &CommandContext) -> Result<()> {
    let balance = ctx
        .client()
        .get_credits()
        .send()
        .await
        .context("fetching credit balance")?
        .into_inner();
    match ctx.format() {
        OutputFormat::Json => print_json(ctx.output(), &balance),
        OutputFormat::Text => {
            println!(
                "subscription: {} (resets with plan)  api top-ups: {}",
                balance.app_subscription, balance.shared_topup
            );
            println!("total: {}", balance.total);
            Ok(())
        }
    }
}

async fn subscription(ctx: &CommandContext) -> Result<()> {
    let subscription = ctx
        .client()
        .get_subscription()
        .send()
        .await
        .context("fetching subscription")?
        .into_inner();
    match ctx.format() {
        OutputFormat::Json => print_json(ctx.output(), &subscription),
        OutputFormat::Text => {
            println!("{} {}", subscription.tier, subscription.status);
            Ok(())
        }
    }
}

async fn portal(args: PortalArgs, ctx: &CommandContext) -> Result<()> {
    let body: nolgia_client::types::PortalLinkRequest =
        nolgia_client::types::PortalLinkRequest::builder()
            .return_url(args.return_url)
            .try_into()
            .context("building portal request")?;
    let portal = ctx
        .client()
        .create_portal_link()
        .body(body)
        .send()
        .await
        .context("creating portal link")?
        .into_inner();
    match ctx.format() {
        OutputFormat::Json => print_json(ctx.output(), &portal),
        OutputFormat::Text => {
            println!("{}", portal.url);
            Ok(())
        }
    }
}
