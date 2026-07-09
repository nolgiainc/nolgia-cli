use anyhow::{Context, Result, ensure};
use clap::{Args, Subcommand};
use nolgia_client::types::{
    AddProjectAssetsRequest, CreateProjectRequest, CreateProjectRequestAutoTagsItem,
    CreateProjectRequestDescription, UpdateProjectRequest, UpdateProjectRequestAutoTagsItem,
    UpdateProjectRequestDescription, UpdateProjectRequestName,
};
use uuid::Uuid;

use crate::output::{OutputFormat, print_json};

use super::CommandContext;

#[derive(Subcommand, Debug)]
pub enum ProjectsCommand {
    /// List your projects, newest first
    List,
    /// Fetch one project
    Get(GetProjectArgs),
    /// Create a project to group assets
    Create(CreateProjectArgs),
    /// Update a project; only the provided fields change
    Update(UpdateProjectArgs),
    /// Delete a project (member assets are never deleted)
    Delete(DeleteProjectArgs),
    /// Add existing assets to a project (idempotent)
    AddAssets(AddAssetsArgs),
    /// Remove an asset from a project (the asset itself is not deleted)
    RemoveAsset(RemoveAssetArgs),
}

#[derive(Args, Debug)]
pub struct GetProjectArgs {
    pub project_id: Uuid,
}

#[derive(Args, Debug)]
pub struct CreateProjectArgs {
    #[arg(long)]
    pub name: String,
    #[arg(long)]
    pub description: Option<String>,
    /// Auto-tag: new assets carrying this tag are added to the project automatically
    /// (repeat for multiple, up to 10).
    #[arg(long = "auto-tag", value_name = "TAG")]
    pub auto_tag: Vec<String>,
}

#[derive(Args, Debug)]
pub struct UpdateProjectArgs {
    pub project_id: Uuid,
    #[arg(long)]
    pub name: Option<String>,
    #[arg(long)]
    pub description: Option<String>,
    /// Replace the project's full auto-tag set (repeat for multiple, up to 10).
    /// New assets carrying an overlapping tag are added to the project automatically.
    #[arg(long = "auto-tag", value_name = "TAG")]
    pub auto_tag: Vec<String>,
    /// Remove all auto-tags from the project (cannot be combined with --auto-tag).
    #[arg(long, conflicts_with = "auto_tag")]
    pub clear_auto_tags: bool,
}

#[derive(Args, Debug)]
pub struct DeleteProjectArgs {
    pub project_id: Uuid,
}

#[derive(Args, Debug)]
pub struct AddAssetsArgs {
    pub project_id: Uuid,
    /// Asset id to add (repeatable; at least one required)
    #[arg(long = "asset-id", value_name = "UUID", required = true)]
    pub asset_ids: Vec<Uuid>,
}

#[derive(Args, Debug)]
pub struct RemoveAssetArgs {
    pub project_id: Uuid,
    pub asset_id: Uuid,
}

pub async fn run(command: ProjectsCommand, ctx: &CommandContext) -> Result<()> {
    match command {
        ProjectsCommand::List => list(ctx).await,
        ProjectsCommand::Get(args) => get(args, ctx).await,
        ProjectsCommand::Create(args) => create(args, ctx).await,
        ProjectsCommand::Update(args) => update(args, ctx).await,
        ProjectsCommand::Delete(args) => delete(args, ctx).await,
        ProjectsCommand::AddAssets(args) => add_assets(args, ctx).await,
        ProjectsCommand::RemoveAsset(args) => remove_asset(args, ctx).await,
    }
}

async fn list(ctx: &CommandContext) -> Result<()> {
    let list = ctx
        .client()
        .list_projects()
        .send()
        .await
        .context("listing projects")?
        .into_inner();
    match ctx.format() {
        OutputFormat::Json => print_json(&list),
        OutputFormat::Text => {
            for project in list.projects {
                print_project_line(&project);
            }
            Ok(())
        }
    }
}

async fn get(args: GetProjectArgs, ctx: &CommandContext) -> Result<()> {
    let project = ctx
        .client()
        .get_project()
        .id(args.project_id)
        .send()
        .await
        .context("fetching project")?
        .into_inner();
    match ctx.format() {
        OutputFormat::Json => print_json(&project),
        OutputFormat::Text => {
            print_project_line(&project);
            Ok(())
        }
    }
}

async fn create(args: CreateProjectArgs, ctx: &CommandContext) -> Result<()> {
    let description: Option<CreateProjectRequestDescription> = args
        .description
        .map(|d| d.parse())
        .transpose()
        .context("invalid --description")?;
    let auto_tags: Vec<CreateProjectRequestAutoTagsItem> = args
        .auto_tag
        .iter()
        .map(|t| t.parse())
        .collect::<Result<_, _>>()
        .context("invalid --auto-tag")?;
    let body: CreateProjectRequest = CreateProjectRequest::builder()
        .name(args.name)
        .description(description)
        .auto_tags(auto_tags)
        .try_into()
        .context("building create-project request")?;
    let project = ctx
        .client()
        .create_project()
        .body(body)
        .send()
        .await
        .context("creating project")?
        .into_inner();
    match ctx.format() {
        OutputFormat::Json => print_json(&project),
        OutputFormat::Text => {
            print_project_line(&project);
            Ok(())
        }
    }
}

async fn update(args: UpdateProjectArgs, ctx: &CommandContext) -> Result<()> {
    let name: Option<UpdateProjectRequestName> = args
        .name
        .map(|n| n.parse())
        .transpose()
        .context("invalid --name")?;
    let description: Option<UpdateProjectRequestDescription> = args
        .description
        .map(|d| d.parse())
        .transpose()
        .context("invalid --description")?;
    // Distinguish "leave auto-tags unchanged" (None) from "clear them"
    // (Some(empty vec)) from "replace with this set" (Some(non-empty)).
    let auto_tags: Option<Vec<UpdateProjectRequestAutoTagsItem>> = if args.clear_auto_tags {
        Some(Vec::new())
    } else if args.auto_tag.is_empty() {
        None
    } else {
        Some(
            args.auto_tag
                .iter()
                .map(|t| t.parse())
                .collect::<Result<_, _>>()
                .context("invalid --auto-tag")?,
        )
    };
    ensure!(
        name.is_some() || description.is_some() || auto_tags.is_some(),
        "provide at least one of --name, --description, --auto-tag, or --clear-auto-tags"
    );
    let body = UpdateProjectRequest {
        name,
        description,
        auto_tags,
    };
    let project = ctx
        .client()
        .update_project()
        .id(args.project_id)
        .body(body)
        .send()
        .await
        .context("updating project")?
        .into_inner();
    match ctx.format() {
        OutputFormat::Json => print_json(&project),
        OutputFormat::Text => {
            print_project_line(&project);
            Ok(())
        }
    }
}

async fn delete(args: DeleteProjectArgs, ctx: &CommandContext) -> Result<()> {
    ctx.client()
        .delete_project()
        .id(args.project_id)
        .send()
        .await
        .context("deleting project")?;
    match ctx.format() {
        OutputFormat::Json => print_json(&serde_json::json!({ "deleted": args.project_id })),
        OutputFormat::Text => {
            println!("deleted {}", args.project_id);
            Ok(())
        }
    }
}

async fn add_assets(args: AddAssetsArgs, ctx: &CommandContext) -> Result<()> {
    let count = args.asset_ids.len();
    let body = AddProjectAssetsRequest {
        asset_ids: args.asset_ids,
    };
    ctx.client()
        .add_project_assets()
        .id(args.project_id)
        .body(body)
        .send()
        .await
        .context("adding assets to project")?;
    match ctx.format() {
        OutputFormat::Json => print_json(&serde_json::json!({
            "project_id": args.project_id,
            "added": count,
        })),
        OutputFormat::Text => {
            println!("added {count} asset(s) to {}", args.project_id);
            Ok(())
        }
    }
}

async fn remove_asset(args: RemoveAssetArgs, ctx: &CommandContext) -> Result<()> {
    ctx.client()
        .remove_project_asset()
        .id(args.project_id)
        .asset_id(args.asset_id)
        .send()
        .await
        .context("removing asset from project")?;
    match ctx.format() {
        OutputFormat::Json => print_json(&serde_json::json!({
            "project_id": args.project_id,
            "removed": args.asset_id,
        })),
        OutputFormat::Text => {
            println!("removed {} from {}", args.asset_id, args.project_id);
            Ok(())
        }
    }
}

fn print_project_line(project: &nolgia_client::types::Project) {
    println!(
        "{} {} ({} asset{})",
        project.id,
        project.name.as_str(),
        project.asset_count,
        if project.asset_count == 1 { "" } else { "s" }
    );
}
