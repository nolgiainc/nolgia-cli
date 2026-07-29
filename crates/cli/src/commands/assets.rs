use anyhow::{Context, Result};
use clap::{Args, Subcommand};
use nolgia_client::ClientExt;
use nolgia_client::types::{Modality, UpdateAssetRequest, UpdateAssetRequestTagsItem};
use std::{num::NonZeroU64, path::PathBuf};
use uuid::Uuid;

use crate::output::{OutputFormat, print_json};

use super::CommandContext;

#[derive(Subcommand, Debug)]
pub enum AssetsCommand {
    List(ListAssetsArgs),
    Get(GetAssetArgs),
    Delete(DeleteAssetArgs),
    /// Upload a local media file (image/video/audio) and get a reusable asset id
    Upload(UploadAssetArgs),
    /// Replace an asset's full tag set
    Tag(TagAssetArgs),
    /// Extract a still frame from a video asset as a new image asset
    Frame(FrameAssetArgs),
}

#[derive(Args, Debug)]
pub struct ListAssetsArgs {
    #[arg(long)]
    pub limit: Option<NonZeroU64>,
    #[arg(long)]
    pub cursor: Option<String>,
    #[arg(long)]
    pub modality: Option<Modality>,
    /// Return only assets carrying the given tag
    #[arg(long)]
    pub tag: Option<String>,
    /// Return only assets belonging to the given project
    #[arg(long)]
    pub project_id: Option<Uuid>,
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

#[derive(Args, Debug)]
pub struct UploadAssetArgs {
    pub file: PathBuf,
    /// File the uploaded asset into this project (`nolgia projects list`
    /// for ids). The project must exist and belong to you.
    #[arg(long, value_name = "PROJECT_UUID")]
    pub project_id: Option<Uuid>,
}

#[derive(Args, Debug)]
pub struct TagAssetArgs {
    pub asset_id: Uuid,
    /// Tag to set (repeat for multiple, up to 10). The full tag set is replaced;
    /// tags are trimmed, lowercased, and de-duplicated server-side.
    #[arg(long, value_name = "TAG")]
    pub tag: Vec<String>,
    /// Remove all tags from the asset (cannot be combined with --tag)
    #[arg(long, conflicts_with = "tag")]
    pub clear: bool,
}

#[derive(Args, Debug)]
#[command(after_help = "Omitting --at extracts the LAST frame — useful as the \
--input (start frame) of a follow-up image-to-video generation to chain \
clips seamlessly.")]
pub struct FrameAssetArgs {
    /// A video asset to extract the frame from
    pub asset_id: Uuid,
    /// Timestamp in seconds (from the start) of the frame to extract
    #[arg(long, value_name = "SECONDS")]
    pub at: Option<f64>,
    /// Extract the last frame (the default when --at is omitted)
    #[arg(long, conflicts_with = "at")]
    pub last: bool,
    /// Also download the extracted frame to this path
    #[arg(long)]
    pub out: Option<PathBuf>,
}

pub async fn run(command: AssetsCommand, ctx: &CommandContext) -> Result<()> {
    match command {
        AssetsCommand::List(args) => list(args, ctx).await,
        AssetsCommand::Get(args) => get(args, ctx).await,
        AssetsCommand::Delete(args) => delete(args, ctx).await,
        AssetsCommand::Upload(args) => upload(args, ctx).await,
        AssetsCommand::Tag(args) => tag(args, ctx).await,
        AssetsCommand::Frame(args) => frame(args, ctx).await,
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
    if let Some(tag) = args.tag {
        request = request.tag(tag);
    }
    if let Some(project_id) = args.project_id {
        request = request.project_id(project_id);
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

async fn tag(args: TagAssetArgs, ctx: &CommandContext) -> Result<()> {
    anyhow::ensure!(
        !args.tag.is_empty() || args.clear,
        "provide at least one --tag, or --clear to remove all tags"
    );
    let asset = if args.clear {
        // The generated UpdateAssetRequest.tags field drops an empty vec on
        // serialization, so the typed builder can't express "clear all tags"
        // (`{"tags": []}`). Use the raw-request helper to send it literally.
        ctx.client()
            .clear_asset_tags(args.asset_id)
            .await
            .context("clearing asset tags")?
    } else {
        let tags: Vec<UpdateAssetRequestTagsItem> = args
            .tag
            .iter()
            .map(|t| t.parse())
            .collect::<Result<_, _>>()
            .context("invalid --tag")?;
        let body: UpdateAssetRequest = UpdateAssetRequest::builder()
            .tags(tags)
            .try_into()
            .context("building tag request")?;
        ctx.client()
            .update_asset()
            .id(args.asset_id)
            .body(body)
            .send()
            .await
            .context("tagging asset")?
            .into_inner()
    };
    match ctx.format() {
        OutputFormat::Json => print_json(&asset),
        OutputFormat::Text => {
            let tags: Vec<&str> = asset.tags.iter().map(|t| t.as_str()).collect();
            println!("{} tags: [{}]", asset.id, tags.join(", "));
            Ok(())
        }
    }
}

async fn frame(args: FrameAssetArgs, ctx: &CommandContext) -> Result<()> {
    let mut request = ctx.client().extract_asset_frame().id(args.asset_id);
    if let Some(t_seconds) = args.at {
        request = request.body_map(|body| body.t_seconds(Some(t_seconds)));
    }
    let asset = match request.send().await {
        Ok(response) => response.into_inner(),
        Err(err) => return Err(super::api_error(err, "extracting frame").await),
    };
    if let Some(out) = args.out.as_ref() {
        super::r#gen::download(&asset.signed_url, out).await?;
    }
    match ctx.format() {
        OutputFormat::Json => print_json(&asset),
        OutputFormat::Text => {
            println!("{} {} {}", asset.id, asset.modality, asset.signed_url);
            if let Some(out) = args.out {
                println!("wrote {}", out.display());
            }
            Ok(())
        }
    }
}

async fn upload(args: UploadAssetArgs, ctx: &CommandContext) -> Result<()> {
    let asset = super::r#gen::upload_asset_file(&args.file, ctx, args.project_id).await?;
    match ctx.format() {
        OutputFormat::Json => print_json(&asset),
        OutputFormat::Text => {
            println!("{} {} {}", asset.id, asset.modality, asset.signed_url);
            Ok(())
        }
    }
}
