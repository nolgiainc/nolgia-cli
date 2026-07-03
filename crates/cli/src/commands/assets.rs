use anyhow::{Context, Result};
use clap::{Args, Subcommand};
use nolgia_client::types::Modality;
use std::{num::NonZeroU64, path::PathBuf};
use uuid::Uuid;

use crate::output::{OutputFormat, print_json};

use super::CommandContext;

#[derive(Subcommand, Debug)]
pub enum AssetsCommand {
    List(ListAssetsArgs),
    Get(GetAssetArgs),
    Delete(DeleteAssetArgs),
}

#[derive(Args, Debug)]
pub struct ListAssetsArgs {
    #[arg(long)]
    pub limit: Option<NonZeroU64>,
    #[arg(long)]
    pub cursor: Option<String>,
    #[arg(long)]
    pub modality: Option<Modality>,
}

#[derive(Args, Debug)]
pub struct GetAssetArgs {
    pub asset_id: Uuid,
    #[arg(long)]
    pub out: Option<PathBuf>,
}

#[derive(Args, Debug)]
pub struct DeleteAssetArgs {
    pub asset_id: Uuid,
}

pub async fn run(command: AssetsCommand, ctx: &CommandContext) -> Result<()> {
    match command {
        AssetsCommand::List(args) => list(args, ctx).await,
        AssetsCommand::Get(args) => get(args, ctx).await,
        AssetsCommand::Delete(args) => delete(args, ctx).await,
    }
}

async fn list(args: ListAssetsArgs, ctx: &CommandContext) -> Result<()> {
    let mut request = ctx.client().list_assets();
    if let Some(limit) = args.limit {
        request = request.limit(limit);
    }
    if let Some(cursor) = args.cursor {
        request = request.cursor(cursor);
    }
    if let Some(modality) = args.modality {
        request = request.modality(modality);
    }
    let page = request.send().await.context("listing assets")?.into_inner();

    match ctx.format() {
        OutputFormat::Json => print_json(&page),
        OutputFormat::Text => {
            for asset in page.items {
                println!("{} {} {}", asset.id, asset.modality, asset.signed_url);
            }
            Ok(())
        }
    }
}

async fn get(args: GetAssetArgs, ctx: &CommandContext) -> Result<()> {
    let asset = ctx
        .client()
        .get_asset()
        .id(args.asset_id)
        .send()
        .await
        .context("fetching asset")?
        .into_inner();

    if let Some(out) = args.out {
        super::r#gen::download(&asset.signed_url, &out).await?;
        match ctx.format() {
            OutputFormat::Json => print_json(&serde_json::json!({"asset": asset, "wrote": out})),
            OutputFormat::Text => {
                println!("wrote {}", out.display());
                Ok(())
            }
        }
    } else {
        match ctx.format() {
            OutputFormat::Json => print_json(&asset),
            OutputFormat::Text => {
                println!(
                    "{} {} {} {}",
                    asset.id, asset.modality, asset.model, asset.signed_url
                );
                Ok(())
            }
        }
    }
}

async fn delete(args: DeleteAssetArgs, ctx: &CommandContext) -> Result<()> {
    ctx.client()
        .delete_asset()
        .id(args.asset_id)
        .send()
        .await
        .context("deleting asset")?;
    match ctx.format() {
        OutputFormat::Json => print_json(&serde_json::json!({ "deleted": args.asset_id })),
        OutputFormat::Text => {
            println!("deleted {}", args.asset_id);
            Ok(())
        }
    }
}
