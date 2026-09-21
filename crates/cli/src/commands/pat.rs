use anyhow::{Context, Result};
use clap::{Args, Subcommand};
use uuid::Uuid;

use crate::output::{OutputFormat, print_json};

use super::CommandContext;

#[derive(Subcommand, Debug)]
pub enum PatCommand {
    Create(CreatePatArgs),
    List,
    Revoke(RevokePatArgs),
}

#[derive(Args, Debug)]
pub struct CreatePatArgs {
    #[arg(long)]
    pub name: String,
}

#[derive(Args, Debug)]
pub struct RevokePatArgs {
    pub pat_id: Uuid,
}

pub async fn run(command: PatCommand, ctx: &CommandContext) -> Result<()> {
    match command {
        PatCommand::Create(args) => create(args, ctx).await,
        PatCommand::List => list(ctx).await,
        PatCommand::Revoke(args) => revoke(args, ctx).await,
    }
}

async fn create(args: CreatePatArgs, ctx: &CommandContext) -> Result<()> {
    let body: nolgia_client::types::CreatePatRequest =
        nolgia_client::types::CreatePatRequest::builder()
            .name(args.name)
            .try_into()
            .context("building create-pat request")?;
    let created = ctx
        .client()
        .create_pat()
        .body(body)
        .send()
        .await
        .context("creating personal access token")?
        .into_inner();
    match ctx.format() {
        OutputFormat::Json => print_json(ctx.output(), &created),
        OutputFormat::Text => {
            println!("created {} ({})", created.pat.id, created.pat.name.as_str());
            println!("token: {}", created.token);
            println!("warning: this token will not be shown again; store it securely");
            Ok(())
        }
    }
}

async fn list(ctx: &CommandContext) -> Result<()> {
    let page = ctx
        .client()
        .list_pats()
        .send()
        .await
        .context("listing personal access tokens")?
        .into_inner();
    match ctx.format() {
        OutputFormat::Json => print_json(ctx.output(), &page),
        OutputFormat::Text => {
            for pat in page.items {
                let last_used = pat
                    .last_used_at
                    .map(|at| at.to_rfc3339())
                    .unwrap_or_else(|| "never".to_string());
                println!(
                    "{} {} {} created {} last used {}",
                    pat.id,
                    pat.name.as_str(),
                    pat.prefix.as_str(),
                    pat.created_at.to_rfc3339(),
                    last_used
                );
            }
            Ok(())
        }
    }
}

async fn revoke(args: RevokePatArgs, ctx: &CommandContext) -> Result<()> {
    ctx.client()
        .revoke_pat()
        .id(args.pat_id)
        .send()
        .await
        .context("revoking personal access token")?;
    match ctx.format() {
        OutputFormat::Json => {
            print_json(ctx.output(), &serde_json::json!({ "revoked": args.pat_id }))
        }
        OutputFormat::Text => {
            println!("revoked {}", args.pat_id);
            Ok(())
        }
    }
}
