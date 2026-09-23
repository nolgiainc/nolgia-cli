use std::io::{self, BufRead, IsTerminal, Write};

use anyhow::{Context, Result, bail, ensure};
use clap::{Args, Subcommand};
use nolgia_client::types::{
    CreateCharacterRequest, CreateCharacterRequestDescription, UpdateCharacterRequest,
    UpdateCharacterRequestDescription, UpdateCharacterRequestName,
};
use uuid::Uuid;

use crate::agent_guard::AgentRefused;
use crate::output::{OutputFormat, print_json};

use super::CommandContext;

/// The consent to the face identity check (NOL-1150). nolgia-api records
/// FACE_CHECK_CONSENT_VERSION with every consent, so these words MUST stay
/// identical to its FaceCheckConsentText for the same version; change both
/// (and the web app's copy) together and bump the version.
pub const FACE_CHECK_CONSENT_VERSION: &str = "2026-09-23";
pub const FACE_CHECK_CONSENT_TEXT: &str = "I am the person in these photos, or I have their permission to use them. I agree that NOLGIA checks the faces in these photos and compares them with the faces in the images and videos I make with this character, as described in the Privacy Policy.";
const FACE_CHECK_PRIVACY_URL: &str = "https://nolgia.ai/privacy#face-check";

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
    #[command(flatten)]
    pub consent: FaceCheckConsentArgs,
}

/// Consent to the face identity check for a character's photos. Without it
/// the photos are still used as references; NOLGIA just does not check that
/// the results look like the person.
#[derive(Args, Debug, Default)]
pub struct FaceCheckConsentArgs {
    /// Turn on the face identity check for the character's photos: shows the
    /// consent wording and asks you to type yes. Only you can give it.
    #[arg(long)]
    pub face_check_consent: bool,
    /// With --face-check-consent: agree to the wording without the prompt,
    /// for scripts. Use it only if you have read the wording.
    #[arg(long, requires = "face_check_consent")]
    pub yes: bool,
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
    #[command(flatten)]
    pub consent: FaceCheckConsentArgs,
    /// Withdraw consent to the face identity check for every photo of the
    /// character; the photos stay, the face check turns off.
    #[arg(long, conflicts_with = "face_check_consent")]
    pub withdraw_face_check_consent: bool,
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
        OutputFormat::Json => print_json(ctx.output(), &list),
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
        OutputFormat::Json => print_json(ctx.output(), &character),
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
    let face_check_consent = if args.consent.face_check_consent {
        ensure!(
            !args.reference_asset_ids.is_empty(),
            "--face-check-consent covers the character's photos: add at least one --reference-asset-id"
        );
        confirm_face_check_consent(ctx, args.consent.yes)?;
        Some(true)
    } else {
        None
    };
    let body: CreateCharacterRequest = CreateCharacterRequest::builder()
        .name(args.name)
        .description(description)
        .reference_asset_ids(args.reference_asset_ids)
        .face_check_consent(face_check_consent)
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
        OutputFormat::Json => print_json(ctx.output(), &character),
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
    let face_check_consent = if args.consent.face_check_consent {
        confirm_face_check_consent(ctx, args.consent.yes)?;
        Some(true)
    } else if args.withdraw_face_check_consent {
        Some(false)
    } else {
        None
    };
    ensure!(
        name.is_some()
            || description.is_some()
            || reference_asset_ids.is_some()
            || face_check_consent.is_some(),
        "provide at least one of --name, --description, --reference-asset-id, --face-check-consent, or --withdraw-face-check-consent"
    );
    // `..Default::default()` leaves the Aura identity fields
    // (canonical_description, primary_reference_asset_id) unset: they are
    // patch-semantics optionals this command has no flags for yet, and
    // omitting them from the body is what leaves them untouched server-side.
    let body = UpdateCharacterRequest {
        name,
        description,
        reference_asset_ids,
        face_check_consent,
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
        OutputFormat::Json => print_json(ctx.output(), &character),
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
        OutputFormat::Json => print_json(
            ctx.output(),
            &serde_json::json!({ "deleted": args.character_id }),
        ),
        OutputFormat::Text => {
            println!("deleted {}", args.character_id);
            Ok(())
        }
    }
}

fn print_character_line(character: &nolgia_client::types::Character) {
    println!(
        "{} {} ({} reference{}){}",
        character.id,
        character.name.as_str(),
        character.reference_assets.len(),
        if character.reference_assets.len() == 1 {
            ""
        } else {
            "s"
        },
        face_check_note(character)
    );
}

/// The face check status after a character line: nothing for a character
/// without photos (or an API that does not report it).
fn face_check_note(character: &nolgia_client::types::Character) -> &'static str {
    match &character.face_check {
        Some(_) if character.reference_assets.is_empty() => "",
        Some(check) if check.needs_consent => {
            " · face check off, needs consent (nolgia characters update <id> --face-check-consent)"
        }
        Some(check) if check.enabled => " · face check on",
        Some(_) => " · face check off",
        None => "",
    }
}

/// Shows the consent wording and asks for an explicit yes, on stderr so JSON
/// output stays clean. The platform agent is refused (the API refuses it
/// too); a non-interactive run must pass --yes.
fn confirm_face_check_consent(ctx: &CommandContext, yes: bool) -> Result<()> {
    if ctx.agent().is_some() {
        return Err(AgentRefused::FaceCheckConsent.into());
    }
    let stdin = io::stdin();
    let interactive = stdin.is_terminal();
    confirm_face_check_consent_with(yes, interactive, &mut stdin.lock(), &mut io::stderr())
}

fn confirm_face_check_consent_with(
    yes: bool,
    interactive: bool,
    input: &mut dyn BufRead,
    out: &mut dyn Write,
) -> Result<()> {
    writeln!(
        out,
        "Face check consent (wording version {FACE_CHECK_CONSENT_VERSION}):\n\n  {FACE_CHECK_CONSENT_TEXT}\n\n  Privacy Policy: {FACE_CHECK_PRIVACY_URL}\n"
    )?;
    if yes {
        writeln!(out, "Agreed with --yes.")?;
        return Ok(());
    }
    if !interactive {
        bail!(
            "--face-check-consent needs your answer: run it in a terminal, or add --yes to agree to the wording above. Nothing was changed."
        );
    }
    write!(out, "Type yes to agree: ")?;
    out.flush()?;
    let mut answer = String::new();
    input.read_line(&mut answer)?;
    ensure!(
        answer.trim().eq_ignore_ascii_case("yes"),
        "consent not given, so nothing was changed"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn consent_prompt_accepts_only_an_explicit_yes() {
        for (answer, ok) in [
            ("yes\n", true),
            ("YES\n", true),
            (" yes \n", true),
            ("y\n", false),
            ("\n", false),
            ("no\n", false),
        ] {
            let mut out = Vec::new();
            let result =
                confirm_face_check_consent_with(false, true, &mut answer.as_bytes(), &mut out);
            assert_eq!(result.is_ok(), ok, "answer {answer:?}");
            let shown = String::from_utf8(out).unwrap();
            assert!(
                shown.contains(FACE_CHECK_CONSENT_TEXT),
                "the wording is shown before asking"
            );
            assert!(shown.contains(FACE_CHECK_PRIVACY_URL));
            assert!(shown.contains(FACE_CHECK_CONSENT_VERSION));
        }
    }

    #[test]
    fn consent_prompt_never_assumes_yes_without_a_terminal() {
        let mut out = Vec::new();
        let err = confirm_face_check_consent_with(false, false, &mut "yes\n".as_bytes(), &mut out)
            .unwrap_err();
        assert!(err.to_string().contains("--yes"));
        let mut out = Vec::new();
        assert!(confirm_face_check_consent_with(true, false, &mut "".as_bytes(), &mut out).is_ok());
        assert!(
            String::from_utf8(out)
                .unwrap()
                .contains(FACE_CHECK_CONSENT_TEXT),
            "--yes still shows the wording"
        );
    }

    #[test]
    fn consent_wording_matches_the_api_version() {
        assert_eq!(FACE_CHECK_CONSENT_VERSION, "2026-09-23");
        assert_eq!(
            FACE_CHECK_CONSENT_TEXT,
            "I am the person in these photos, or I have their permission to use them. I agree that NOLGIA checks the faces in these photos and compares them with the faces in the images and videos I make with this character, as described in the Privacy Policy."
        );
    }
}
