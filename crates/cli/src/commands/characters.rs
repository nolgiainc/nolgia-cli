use anyhow::{Context, Result, ensure};
use clap::{Args, Subcommand};
use nolgia_client::types::{
    CreateCharacterRequest, CreateCharacterRequestDescription, UpdateCharacterRequest,
    UpdateCharacterRequestDescription, UpdateCharacterRequestName,
};
use uuid::Uuid;

use crate::output::{OutputFormat, print_json};

use super::CommandContext;

// Raised 4 -> 8 in round 39 (TASK 8): the eight-photo character flow uploads
// the prescribed eight frames as one character's references.
const MAX_REFERENCE_ASSETS: usize = 8;

#[derive(Subcommand, Debug)]
pub enum CharactersCommand {
    /// List your characters, newest first
    List,
    /// Fetch one character with fresh signed reference URLs
    Get(GetCharacterArgs),
    /// Create a reusable character from existing image assets
    Create(CreateCharacterArgs),
    /// Update a character; only the provided fields change
    Update(UpdateCharacterArgs),
    /// Delete a character (its reference assets are not deleted)
    Delete(DeleteCharacterArgs),
}

#[derive(Args, Debug)]
pub struct GetCharacterArgs {
    pub character_id: Uuid,
}

#[derive(Args, Debug)]
pub struct CreateCharacterArgs {
    #[arg(long)]
    pub name: String,
    #[arg(long)]
    pub description: Option<String>,
    /// Existing image asset id to use as a reference (repeat up to 8 times, in display order)
    #[arg(long = "reference-asset-id", value_name = "UUID")]
    pub reference_asset_ids: Vec<Uuid>,
}

#[derive(Args, Debug)]
pub struct UpdateCharacterArgs {
    pub character_id: Uuid,
    #[arg(long)]
    pub name: Option<String>,
    #[arg(long)]
    pub description: Option<String>,
    /// Replaces the full reference set when provided (repeat up to 8 times, in display order)
    #[arg(long = "reference-asset-id", value_name = "UUID")]
    pub reference_asset_ids: Vec<Uuid>,
}

#[derive(Args, Debug)]
pub struct DeleteCharacterArgs {
    pub character_id: Uuid,
}

pub async fn run(command: CharactersCommand, ctx: &CommandContext) -> Result<()> {
    match command {
        CharactersCommand::List => list(ctx).await,
        CharactersCommand::Get(args) => get(args, ctx).await,
        CharactersCommand::Create(args) => create(args, ctx).await,
        CharactersCommand::Update(args) => update(args, ctx).await,
        CharactersCommand::Delete(args) => delete(args, ctx).await,
    }
}

async fn list(ctx: &CommandContext) -> Result<()> {
    let list = ctx
        .client()
        .list_characters()
        .send()
        .await
        .context("listing characters")?
        .into_inner();
    match ctx.format() {
        OutputFormat::Json => print_json(&list),
        OutputFormat::Text => {
            for character in list.characters {
                print_character_line(&character);
            }
            Ok(())
        }
    }
}

async fn get(args: GetCharacterArgs, ctx: &CommandContext) -> Result<()> {
    let character = ctx
        .client()
        .get_character()
        .id(args.character_id)
        .send()
        .await
        .context("fetching character")?
        .into_inner();
    match ctx.format() {
        OutputFormat::Json => print_json(&character),
        OutputFormat::Text => {
            print_character_line(&character);
            Ok(())
        }
    }
}

async fn create(args: CreateCharacterArgs, ctx: &CommandContext) -> Result<()> {
    ensure!(
        args.reference_asset_ids.len() <= MAX_REFERENCE_ASSETS,
        "at most {MAX_REFERENCE_ASSETS} --reference-asset-id values are allowed"
    );
    let description: Option<CreateCharacterRequestDescription> = args
        .description
        .map(|d| d.parse())
        .transpose()
        .context("invalid --description")?;
    let body: CreateCharacterRequest = CreateCharacterRequest::builder()
        .name(args.name)
        .description(description)
        .reference_asset_ids(args.reference_asset_ids)
        .try_into()
        .context("building create-character request")?;
    let character = ctx
        .client()
        .create_character()
        .body(body)
        .send()
        .await
        .context("creating character")?
        .into_inner();
    match ctx.format() {
        OutputFormat::Json => print_json(&character),
        OutputFormat::Text => {
            print_character_line(&character);
            Ok(())
        }
    }
}

async fn update(args: UpdateCharacterArgs, ctx: &CommandContext) -> Result<()> {
    ensure!(
        args.reference_asset_ids.len() <= MAX_REFERENCE_ASSETS,
        "at most {MAX_REFERENCE_ASSETS} --reference-asset-id values are allowed"
    );
    let name: Option<UpdateCharacterRequestName> = args
        .name
        .map(|n| n.parse())
        .transpose()
        .context("invalid --name")?;
    let description: Option<UpdateCharacterRequestDescription> = args
        .description
        .map(|d| d.parse())
        .transpose()
        .context("invalid --description")?;
    let reference_asset_ids = if args.reference_asset_ids.is_empty() {
        None
    } else {
        Some(args.reference_asset_ids)
    };
    ensure!(
        name.is_some() || description.is_some() || reference_asset_ids.is_some(),
        "provide at least one of --name, --description, or --reference-asset-id"
    );
    // `..Default::default()` leaves the Aura identity fields
    // (canonical_description, primary_reference_asset_id) unset: they are
    // patch-semantics optionals this command has no flags for yet, and
    // omitting them from the body is what leaves them untouched server-side.
    let body = UpdateCharacterRequest {
        name,
        description,
        reference_asset_ids,
        ..Default::default()
    };
    let character = ctx
        .client()
        .update_character()
        .id(args.character_id)
        .body(body)
        .send()
        .await
        .context("updating character")?
        .into_inner();
    match ctx.format() {
        OutputFormat::Json => print_json(&character),
        OutputFormat::Text => {
            print_character_line(&character);
            Ok(())
        }
    }
}

async fn delete(args: DeleteCharacterArgs, ctx: &CommandContext) -> Result<()> {
    ctx.client()
        .delete_character()
        .id(args.character_id)
        .send()
        .await
        .context("deleting character")?;
    match ctx.format() {
        OutputFormat::Json => print_json(&serde_json::json!({ "deleted": args.character_id })),
        OutputFormat::Text => {
            println!("deleted {}", args.character_id);
            Ok(())
        }
    }
}

fn print_character_line(character: &nolgia_client::types::Character) {
    println!(
        "{} {} ({} reference{})",
        character.id,
        character.name.as_str(),
        character.reference_assets.len(),
        if character.reference_assets.len() == 1 {
            ""
        } else {
            "s"
        }
    );
}
