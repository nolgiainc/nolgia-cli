//! Marketplace abilities: registry-backed abilities served by nolgia-api.
//! Distinct from `nolgia skills` (SKILL.md packs bundled in the binary):
//! marketplace abilities are published by Nolgia, installed per agent, and
//! materialized onto the agent pod by `nolgia ability sync` (or the chart's
//! sync initContainer, which does the same thing).

use anyhow::{Context, Result, bail};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use clap::{Args, Subcommand};
use nolgia_client::types::{
    Ability, AbilityMinTier, AbilityVisibility, PublishAbilityRequest, PublishAbilityRequestName,
    PublishAbilityRequestSlug, PublishAbilityRequestVersion,
};
use std::{
    fs,
    io::Read as _,
    path::{Path, PathBuf},
};

use crate::output::{OutputFormat, print_json};

use super::CommandContext;

/// Sync marker inside each materialized ability dir; records the synced
/// version. Dirs without it (hand-authored abilities) are never touched.
const MARKER: &str = ".nolgia-ability.json";

/// Package manifest filename. The marketplace identifies an ability package
/// by its `ability.json`.
const MANIFEST: &str = "ability.json";

#[derive(Subcommand, Debug)]
pub enum AbilityCommand {
    /// List the marketplace catalog visible to this account
    List,
    /// Show one marketplace ability
    Show(SlugArgs),
    /// List abilities installed for this account's agent
    Installed,
    /// Install a marketplace ability for this account's agent
    Install(SlugArgs),
    /// Uninstall a marketplace ability from this account's agent
    Uninstall(SlugArgs),
    /// Materialize installed abilities into a skills directory (what the agent
    /// pod's initContainer runs on boot)
    Sync(SyncArgs),
    /// Scaffold a new ability authoring directory (ability.json, SKILL.md,
    /// payload/)
    Init(InitArgs),
    /// Validate an ability authoring directory and assemble the publishable
    /// package.
    ///
    /// Copies ability.json + SKILL.md and the contents of payload/ (which land
    /// at the package root, next to SKILL.md) into the output directory,
    /// ready for `nolgia ability publish`. The optional manifest field
    /// `python_requirements` (an array of pip requirement strings) is passed
    /// through to the marketplace manifest verbatim.
    Pack(PackArgs),
    /// Publish an ability package directory to the marketplace (admin only).
    ///
    /// The authoring loop is `ability init <slug>` to scaffold, `ability pack
    /// <dir>` to validate and assemble, then `ability publish dist/<slug>`.
    Publish(PublishArgs),
}

#[derive(Args, Debug)]
pub struct SlugArgs {
    pub slug: String,
}

#[derive(Args, Debug)]
pub struct SyncArgs {
    /// Target skills directory (default: $HERMES_HOME/skills, HERMES_HOME
    /// defaulting to /opt/data)
    #[arg(long)]
    pub dir: Option<PathBuf>,
}

#[derive(Args, Debug)]
pub struct InitArgs {
    /// Ability slug: lowercase letters, digits, hyphens (max 64 chars)
    pub slug: String,
    /// Directory to create (default: ./<slug>)
    #[arg(long)]
    pub dir: Option<PathBuf>,
}

#[derive(Args, Debug)]
pub struct PackArgs {
    /// Ability authoring directory (from `nolgia ability init`)
    pub dir: PathBuf,
    /// Output package directory (default: dist/<slug>)
    #[arg(long)]
    pub out: Option<PathBuf>,
}

#[derive(Args, Debug)]
pub struct PublishArgs {
    /// Ability package directory containing ability.json + SKILL.md
    /// (assembled by `nolgia ability pack`)
    pub dir: PathBuf,
}

pub async fn run(command: AbilityCommand, ctx: &CommandContext) -> Result<()> {
    match command {
        AbilityCommand::List => list(ctx).await,
        AbilityCommand::Show(args) => show(args, ctx).await,
        AbilityCommand::Installed => installed(ctx).await,
        AbilityCommand::Install(args) => install(args, ctx).await,
        AbilityCommand::Uninstall(args) => uninstall(args, ctx).await,
        AbilityCommand::Sync(args) => sync(args, ctx).await,
        AbilityCommand::Init(args) => init(args, ctx),
        AbilityCommand::Pack(args) => pack(args, ctx),
        AbilityCommand::Publish(args) => publish(args, ctx).await,
    }
}

async fn list(ctx: &CommandContext) -> Result<()> {
    let abilities = ctx
        .client()
        .list_abilities()
        .send()
        .await
        .context("listing marketplace abilities")?
        .into_inner();
    match ctx.format() {
        OutputFormat::Json => print_json(&abilities),
        OutputFormat::Text => {
            for ability in &abilities {
                print_ability_line(ability);
            }
            Ok(())
        }
    }
}

async fn show(args: SlugArgs, ctx: &CommandContext) -> Result<()> {
    let ability = ctx
        .client()
        .get_ability()
        .slug(&args.slug)
        .send()
        .await
        .context("fetching ability")?
        .into_inner();
    match ctx.format() {
        OutputFormat::Json => print_json(&ability),
        OutputFormat::Text => {
            println!(
                "{} v{}  {}",
                ability.slug, ability.latest_version, ability.name
            );
            println!("  {}", ability.description);
            if !ability.min_tier.is_empty() {
                println!(
                    "  requires plan: {} (entitled: {})",
                    ability.min_tier, ability.entitled
                );
            }
            if !ability.credit_cost_hint.is_empty() {
                println!("  credits: {}", ability.credit_cost_hint);
            }
            if !ability.required_env.is_empty() {
                println!("  env: {}", ability.required_env.join(", "));
            }
            Ok(())
        }
    }
}

async fn installed(ctx: &CommandContext) -> Result<()> {
    let abilities = ctx
        .client()
        .list_agent_abilities()
        .send()
        .await
        .context("listing installed abilities")?
        .into_inner();
    match ctx.format() {
        OutputFormat::Json => print_json(&abilities),
        OutputFormat::Text => {
            for ability in &abilities {
                println!(
                    "{:28} v{:10} {}",
                    ability.slug, ability.latest_version, ability.name
                );
            }
            Ok(())
        }
    }
}

async fn install(args: SlugArgs, ctx: &CommandContext) -> Result<()> {
    let ability = ctx
        .client()
        .install_agent_ability()
        .slug(&args.slug)
        .send()
        .await
        .context("installing ability")?
        .into_inner();
    match ctx.format() {
        OutputFormat::Json => print_json(&ability),
        OutputFormat::Text => {
            println!(
                "installed {} v{} — it lands on the agent pod on its next restart or `nolgia ability sync`",
                ability.slug, ability.latest_version
            );
            Ok(())
        }
    }
}

async fn uninstall(args: SlugArgs, ctx: &CommandContext) -> Result<()> {
    ctx.client()
        .uninstall_agent_ability()
        .slug(&args.slug)
        .send()
        .await
        .context("uninstalling ability")?;
    match ctx.format() {
        OutputFormat::Json => print_json(&serde_json::json!({ "uninstalled": args.slug })),
        OutputFormat::Text => {
            println!("uninstalled {}", args.slug);
            Ok(())
        }
    }
}

// --- sync ---------------------------------------------------------------

#[derive(serde::Serialize)]
struct SyncResult {
    slug: String,
    version: String,
    action: &'static str, // "synced" | "current" | "removed" | "skipped"
}

async fn sync(args: SyncArgs, ctx: &CommandContext) -> Result<()> {
    let root = args.dir.unwrap_or_else(|| {
        let home = std::env::var("HERMES_HOME").unwrap_or_else(|_| "/opt/data".into());
        PathBuf::from(home).join("skills")
    });
    fs::create_dir_all(&root).with_context(|| format!("creating {}", root.display()))?;

    let installed = ctx
        .client()
        .list_agent_abilities()
        .send()
        .await
        .context("listing installed abilities")?
        .into_inner();

    let mut results = Vec::new();
    for ability in &installed {
        let dir = root.join(&ability.slug);
        if marker_version(&dir).as_deref() == Some(ability.latest_version.as_str()) {
            results.push(SyncResult {
                slug: ability.slug.clone(),
                version: ability.latest_version.clone(),
                action: "current",
            });
            continue;
        }
        let content = match ctx
            .client()
            .get_ability_content()
            .slug(&ability.slug)
            .send()
            .await
        {
            Ok(response) => response.into_inner(),
            Err(err) => {
                // Entitlement/visibility is enforced server-side per download;
                // skip rather than fail the whole sync.
                eprintln!("skipping {}: {}", ability.slug, err);
                results.push(SyncResult {
                    slug: ability.slug.clone(),
                    version: ability.latest_version.clone(),
                    action: "skipped",
                });
                continue;
            }
        };
        let raw = BASE64
            .decode(&content.content_base64)
            .with_context(|| format!("decoding content for {}", ability.slug))?;
        materialize(&root, &ability.slug, &content.version, &raw)?;
        results.push(SyncResult {
            slug: ability.slug.clone(),
            version: content.version.clone(),
            action: "synced",
        });
    }

    // Remove marketplace-managed dirs no longer in the install list.
    let keep: Vec<&str> = installed.iter().map(|s| s.slug.as_str()).collect();
    for entry in fs::read_dir(&root)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let dir = entry.path();
        if dir.is_dir()
            && !keep.contains(&name.as_str())
            && let Some(version) = marker_version(&dir)
        {
            fs::remove_dir_all(&dir)
                .with_context(|| format!("removing stale ability {}", dir.display()))?;
            results.push(SyncResult {
                slug: name,
                version,
                action: "removed",
            });
        }
    }

    match ctx.format() {
        OutputFormat::Json => print_json(&results),
        OutputFormat::Text => {
            for r in &results {
                println!("{:8} {} v{}", r.action, r.slug, r.version);
            }
            println!("abilities dir: {}", root.display());
            Ok(())
        }
    }
}

fn marker_version(dir: &Path) -> Option<String> {
    let raw = fs::read_to_string(dir.join(MARKER)).ok()?;
    let value: serde_json::Value = serde_json::from_str(&raw).ok()?;
    Some(value.get("version")?.as_str()?.to_string())
}

/// Extract an ability tarball into a staging dir and atomically swap it in.
fn materialize(root: &Path, slug: &str, version: &str, targz: &[u8]) -> Result<()> {
    let staging = root.join(format!(".{slug}.syncing"));
    let _ = fs::remove_dir_all(&staging);
    fs::create_dir_all(&staging)?;

    let decoder = flate2::read::GzDecoder::new(targz);
    let mut archive = tar::Archive::new(decoder);
    // tar-rs refuses path traversal and absolute paths on unpack.
    archive
        .unpack(&staging)
        .with_context(|| format!("extracting {slug}"))?;

    fs::write(
        staging.join(MARKER),
        serde_json::to_string_pretty(&serde_json::json!({ "slug": slug, "version": version }))?
            + "\n",
    )?;

    let target = root.join(slug);
    let _ = fs::remove_dir_all(&target);
    fs::rename(&staging, &target)
        .with_context(|| format!("activating {} -> {}", staging.display(), target.display()))?;
    Ok(())
}

// --- init / pack (authoring) ---------------------------------------------

/// Server-side decoded package size limit (ability_versions.content).
const MAX_PACKAGE_BYTES: u64 = 5 * 1024 * 1024;

/// Server-side slug rule (PublishAbilityRequest.slug: ^[a-z0-9][a-z0-9-]{0,63}$).
fn is_valid_slug(slug: &str) -> bool {
    (1..=64).contains(&slug.len())
        && slug
            .bytes()
            .enumerate()
            .all(|(i, b)| b.is_ascii_lowercase() || b.is_ascii_digit() || (i > 0 && b == b'-'))
}

/// Server-side version rule (PublishAbilityRequest.version: ^[0-9]+\.[0-9]+\.[0-9]+$).
fn is_valid_version(version: &str) -> bool {
    let numeric =
        |part: &&str| -> bool { !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()) };
    let parts: Vec<&str> = version.split('.').collect();
    parts.len() == 3 && parts.iter().all(numeric)
}

fn title_case(slug: &str) -> String {
    slug.split('-')
        .filter(|word| !word.is_empty())
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => first.to_ascii_uppercase().to_string() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[derive(serde::Serialize)]
struct InitResult {
    slug: String,
    dir: PathBuf,
    files: Vec<String>,
}

fn init(args: InitArgs, ctx: &CommandContext) -> Result<()> {
    let dir = args.dir.unwrap_or_else(|| PathBuf::from(&args.slug));
    let result = scaffold(&args.slug, &dir)?;
    match ctx.format() {
        OutputFormat::Json => print_json(&result),
        OutputFormat::Text => {
            println!("initialized {} at {}", result.slug, result.dir.display());
            println!(
                "  ability.json  marketplace manifest — fill in description/visibility/min_tier"
            );
            println!("  SKILL.md      agent-facing instructions (frontmatter + body)");
            println!(
                "  payload/      optional code; packaged at the ability root next to SKILL.md"
            );
            println!();
            println!("next: edit ability.json + SKILL.md, add code under payload/, then");
            println!("  nolgia ability pack {}", result.dir.display());
            Ok(())
        }
    }
}

fn scaffold(slug: &str, dir: &Path) -> Result<InitResult> {
    if !is_valid_slug(slug) {
        bail!(
            "invalid slug {slug:?} — must match ^[a-z0-9][a-z0-9-]{{0,63}}$ \
             (lowercase letters, digits, hyphens)"
        );
    }
    if dir.exists() {
        bail!("{} already exists — refusing to overwrite", dir.display());
    }
    fs::create_dir_all(dir.join("payload"))
        .with_context(|| format!("creating {}", dir.display()))?;
    let name = title_case(slug);
    fs::write(dir.join(MANIFEST), manifest_template(slug, &name))?;
    fs::write(dir.join("SKILL.md"), skill_md_template(slug, &name))?;
    fs::write(dir.join("payload").join(".gitkeep"), "")?;
    Ok(InitResult {
        slug: slug.to_string(),
        dir: dir.to_path_buf(),
        files: vec![
            MANIFEST.into(),
            "SKILL.md".into(),
            "payload/.gitkeep".into(),
        ],
    })
}

fn manifest_template(slug: &str, name: &str) -> String {
    format!(
        r#"{{
  "slug": "{slug}",
  "name": "{name}",
  "version": "0.1.0",
  "description": "TODO: one paragraph shown in the marketplace catalog — what the ability does and when to install it.",
  "required_env": ["NOLGIA_TOKEN", "NOLGIA_API_URL"],
  "credit_cost_hint": "TODO: how this ability spends credits (or note that it is free to use).",
  "min_tier": "",
  "visibility": "private",
  "python_requirements": []
}}
"#
    )
}

fn skill_md_template(slug: &str, name: &str) -> String {
    format!(
        r#"---
name: {slug}
description: |
  TODO: one or two sentences the agent uses to decide when to reach for this
  ability — what it does and the trigger situations.
version: 0.1.0
author: TODO
license: proprietary
metadata:
  tags: [nolgia]
---

# {name}

TODO: short overview. After install + sync this file lands at
$HERMES_HOME/skills/{slug}/SKILL.md and the agent reads it at session start.

## When to use

- TODO: situations where the agent should apply this ability.
- TODO: situations where it should not.

## How

TODO: step-by-step instructions. Files under payload/ are packaged at the
ability's directory root (next to this file), so reference them relative to
the ability directory, e.g.:

```bash
cd "${{HERMES_HOME:-/opt/data}}/skills/{slug}"
python3 tool.py --help
```

## Cost notes

TODO: which steps spend credits and rough magnitude. Anything over ~2k
credits needs human confirmation before running.
"#
    )
}

#[derive(Debug, serde::Serialize)]
struct PackResult {
    slug: String,
    version: String,
    out: PathBuf,
    files: Vec<String>,
    total_bytes: u64,
}

fn pack(args: PackArgs, ctx: &CommandContext) -> Result<()> {
    let result = assemble(&args.dir, args.out)?;
    match ctx.format() {
        OutputFormat::Json => print_json(&result),
        OutputFormat::Text => {
            println!(
                "packed {} v{} -> {} ({} files, {} bytes)",
                result.slug,
                result.version,
                result.out.display(),
                result.files.len(),
                result.total_bytes
            );
            for file in &result.files {
                println!("  {file}");
            }
            println!(
                "publish with: nolgia ability publish {}",
                result.out.display()
            );
            Ok(())
        }
    }
}

/// Validate an authoring directory and assemble the package `ability publish`
/// consumes: ability.json + SKILL.md + payload/ contents at the package root.
fn assemble(dir: &Path, out: Option<PathBuf>) -> Result<PackResult> {
    let manifest_path = dir.join(MANIFEST);
    let manifest_raw = fs::read_to_string(&manifest_path).with_context(|| {
        format!(
            "reading {} — scaffold an authoring directory with `nolgia ability init`",
            manifest_path.display()
        )
    })?;
    let manifest: serde_json::Map<String, serde_json::Value> =
        serde_json::from_str(&manifest_raw).context("parsing ability.json")?;
    validate_manifest(&manifest)?;
    let slug = manifest["slug"].as_str().unwrap_or_default().to_string();
    let version = manifest["version"].as_str().unwrap_or_default().to_string();

    if !dir.join("SKILL.md").is_file() {
        bail!(
            "{} has no SKILL.md — every ability needs agent-facing instructions",
            dir.display()
        );
    }

    let out = out.unwrap_or_else(|| PathBuf::from("dist").join(&slug));
    if out.exists() {
        fs::remove_dir_all(&out).with_context(|| format!("clearing {}", out.display()))?;
    }
    fs::create_dir_all(&out).with_context(|| format!("creating {}", out.display()))?;
    fs::copy(dir.join(MANIFEST), out.join(MANIFEST))?;
    fs::copy(dir.join("SKILL.md"), out.join("SKILL.md"))?;
    let payload = dir.join("payload");
    if payload.is_dir() {
        copy_payload(&payload, &out, true).inspect_err(|_| {
            let _ = fs::remove_dir_all(&out);
        })?;
    }

    let mut files = Vec::new();
    let mut total_bytes = 0u64;
    collect_files(&out, Path::new(""), &mut files, &mut total_bytes)?;
    files.sort();
    if total_bytes > MAX_PACKAGE_BYTES {
        let _ = fs::remove_dir_all(&out);
        bail!(
            "package is {total_bytes} bytes decoded — the marketplace limit is 5 MiB \
             ({MAX_PACKAGE_BYTES} bytes); trim payload/"
        );
    }
    Ok(PackResult {
        slug,
        version,
        out,
        files,
        total_bytes,
    })
}

fn validate_manifest(manifest: &serde_json::Map<String, serde_json::Value>) -> Result<()> {
    let string_field = |key: &str| -> Result<&str> {
        manifest
            .get(key)
            .and_then(|v| v.as_str())
            .with_context(|| format!("ability.json is missing required string field {key:?}"))
    };
    let slug = string_field("slug")?;
    if !is_valid_slug(slug) {
        bail!(
            "ability.json slug {slug:?} is invalid — must match ^[a-z0-9][a-z0-9-]{{0,63}}$ \
             (lowercase letters, digits, hyphens)"
        );
    }
    let name = string_field("name")?;
    if name.is_empty() || name.len() > 128 {
        bail!("ability.json name must be 1..=128 characters");
    }
    let version = string_field("version")?;
    if !is_valid_version(version) {
        bail!("ability.json version {version:?} is invalid — must be semver digits like \"1.0.0\"");
    }
    if string_field("description")?.is_empty() {
        bail!("ability.json description must not be empty — it is the marketplace listing text");
    }
    if let Some(value) = manifest.get("visibility")
        && !matches!(value.as_str(), Some("public") | Some("private"))
    {
        bail!("ability.json visibility must be \"public\" or \"private\", got {value}");
    }
    const TIERS: &[&str] = &[
        "",
        "starter",
        "pro",
        "studio",
        "studio_min",
        "studio_mid",
        "studio_max",
    ];
    if let Some(value) = manifest.get("min_tier")
        && !value.as_str().is_some_and(|t| TIERS.contains(&t))
    {
        bail!(
            "ability.json min_tier {value} is not a subscription tier (empty for free, or {})",
            TIERS[1..].join(", ")
        );
    }
    for key in ["required_env", "python_requirements"] {
        if let Some(value) = manifest.get(key) {
            let ok = value.as_array().is_some_and(|items| {
                items
                    .iter()
                    .all(|item| item.as_str().is_some_and(|s| !s.is_empty()))
            });
            if !ok {
                bail!("ability.json {key} must be an array of non-empty strings");
            }
        }
    }
    Ok(())
}

/// Copy payload/ contents into the package root, skipping scaffolding and
/// cache noise. Top-level names may not shadow the manifest or SKILL.md.
fn copy_payload(src: &Path, dst: &Path, top: bool) -> Result<()> {
    fs::create_dir_all(dst)?;
    let mut entries: Vec<_> = fs::read_dir(src)?.collect::<std::io::Result<_>>()?;
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name == "__pycache__" || name == ".gitkeep" || name == ".git" {
            continue;
        }
        if top && (name == MANIFEST || name == "SKILL.md") {
            bail!("payload/{name} would overwrite the package {name} — rename it");
        }
        let from = entry.path();
        let to = dst.join(&name);
        if from.is_dir() {
            copy_payload(&from, &to, false)?;
        } else {
            fs::copy(&from, &to).with_context(|| format!("copying {}", from.display()))?;
        }
    }
    Ok(())
}

fn collect_files(
    dir: &Path,
    prefix: &Path,
    files: &mut Vec<String>,
    total: &mut u64,
) -> Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let rel = prefix.join(entry.file_name());
        let path = entry.path();
        if path.is_dir() {
            collect_files(&path, &rel, files, total)?;
        } else {
            *total += entry.metadata()?.len();
            files.push(rel.to_string_lossy().into_owned());
        }
    }
    Ok(())
}

// --- publish ------------------------------------------------------------

async fn publish(args: PublishArgs, ctx: &CommandContext) -> Result<()> {
    let manifest_path = args.dir.join(MANIFEST);
    let manifest_raw = fs::read_to_string(&manifest_path)
        .with_context(|| format!("reading {}", manifest_path.display()))?;
    let manifest: serde_json::Map<String, serde_json::Value> =
        serde_json::from_str(&manifest_raw).context("parsing ability.json")?;
    if !args.dir.join("SKILL.md").is_file() {
        bail!(
            "{} has no SKILL.md — not an ability package",
            args.dir.display()
        );
    }

    let field = |key: &str| -> Result<String> {
        manifest
            .get(key)
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .with_context(|| format!("ability.json is missing required string field {key:?}"))
    };
    let slug: PublishAbilityRequestSlug = field("slug")?.parse().context("invalid slug")?;
    let name: PublishAbilityRequestName = field("name")?.parse().context("invalid name")?;
    let version: PublishAbilityRequestVersion =
        field("version")?.parse().context("invalid version")?;
    let description = manifest
        .get("description")
        .and_then(|v| v.as_str())
        .map(|d| d.parse())
        .transpose()
        .context("invalid description")?;
    let credit_cost_hint = manifest
        .get("credit_cost_hint")
        .and_then(|v| v.as_str())
        .map(|h| h.parse())
        .transpose()
        .context("invalid credit_cost_hint")?;
    let required_env: Vec<String> = manifest
        .get("required_env")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    let min_tier: Option<AbilityMinTier> = manifest
        .get("min_tier")
        .and_then(|v| v.as_str())
        .map(|t| serde_json::from_value(serde_json::Value::String(t.to_string())))
        .transpose()
        .context("ability.json min_tier is not a valid subscription tier")?;
    let visibility: Option<AbilityVisibility> = manifest
        .get("visibility")
        .and_then(|v| v.as_str())
        .map(|t| serde_json::from_value(serde_json::Value::String(t.to_string())))
        .transpose()
        .context("ability.json visibility must be \"public\" or \"private\"")?;

    let content_base64 = BASE64.encode(pack_targz(&args.dir)?);

    let body = PublishAbilityRequest {
        slug,
        name,
        version,
        description,
        credit_cost_hint,
        required_env,
        min_tier,
        access: None,
        price_cents: None,
        interval: None,
        visibility,
        manifest,
        content_base64,
    };
    let ability = ctx
        .client()
        .publish_ability()
        .body(body)
        .send()
        .await
        .context("publishing ability (admin only)")?
        .into_inner();
    match ctx.format() {
        OutputFormat::Json => print_json(&ability),
        OutputFormat::Text => {
            println!(
                "published {} v{} ({}, min_tier: {})",
                ability.slug,
                ability.latest_version,
                ability.visibility,
                if ability.min_tier.is_empty() {
                    "free"
                } else {
                    &ability.min_tier
                }
            );
            Ok(())
        }
    }
}

/// Tar+gzip the package directory (contents at the archive root).
fn pack_targz(dir: &Path) -> Result<Vec<u8>> {
    let encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    let mut builder = tar::Builder::new(encoder);
    append_dir(&mut builder, dir, Path::new(""))?;
    Ok(builder.into_inner()?.finish()?)
}

fn append_dir<W: std::io::Write>(
    builder: &mut tar::Builder<W>,
    dir: &Path,
    prefix: &Path,
) -> Result<()> {
    let mut entries: Vec<_> = fs::read_dir(dir)?.collect::<std::io::Result<_>>()?;
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let name = entry.file_name();
        if name.to_string_lossy() == "__pycache__" {
            continue;
        }
        let path = entry.path();
        let archived = prefix.join(&name);
        if path.is_dir() {
            append_dir(builder, &path, &archived)?;
        } else {
            let mut file = fs::File::open(&path)?;
            let mut buf = Vec::new();
            file.read_to_end(&mut buf)?;
            let mut header = tar::Header::new_gnu();
            header.set_size(buf.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            builder.append_data(&mut header, &archived, buf.as_slice())?;
        }
    }
    Ok(())
}

fn print_ability_line(ability: &Ability) {
    let mut tags = Vec::new();
    if ability.visibility == AbilityVisibility::Private.to_string() {
        tags.push("private".to_string());
    }
    if !ability.min_tier.is_empty() {
        tags.push(format!("requires {}", ability.min_tier));
    }
    if !ability.entitled {
        tags.push("not entitled".to_string());
    }
    let suffix = if tags.is_empty() {
        String::new()
    } else {
        format!("  [{}]", tags.join(", "))
    };
    println!(
        "{:28} v{:10} {}{}",
        ability.slug, ability.latest_version, ability.name, suffix
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pack_and_materialize_roundtrip() {
        let base = std::env::temp_dir().join(format!("nolgia-ability-test-{}", std::process::id()));
        let src = base.join("src/my-ability");
        fs::create_dir_all(src.join("nested")).unwrap();
        fs::write(src.join("SKILL.md"), "---\nname: my-ability\n---\n").unwrap();
        fs::write(src.join("nested/tool.py"), "print('hi')\n").unwrap();
        fs::create_dir_all(src.join("__pycache__")).unwrap();
        fs::write(src.join("__pycache__/junk.pyc"), "x").unwrap();

        let targz = pack_targz(&src).unwrap();
        let root = base.join("skills");
        fs::create_dir_all(&root).unwrap();
        materialize(&root, "my-ability", "1.2.3", &targz).unwrap();

        let installed = root.join("my-ability");
        assert!(installed.join("SKILL.md").is_file());
        assert!(installed.join("nested/tool.py").is_file());
        assert!(!installed.join("__pycache__").exists());
        assert_eq!(marker_version(&installed).as_deref(), Some("1.2.3"));

        // Re-materializing a new version replaces the dir atomically.
        materialize(&root, "my-ability", "1.3.0", &targz).unwrap();
        assert_eq!(marker_version(&installed).as_deref(), Some("1.3.0"));

        fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn slug_and_version_rules_match_the_publish_api() {
        for slug in ["a", "short-film", "x9", "a-b-c-1", &"a".repeat(64)] {
            assert!(is_valid_slug(slug), "expected valid: {slug}");
        }
        for slug in ["", "-x", "Short-Film", "a_b", "a b", "é", &"a".repeat(65)] {
            assert!(!is_valid_slug(slug), "expected invalid: {slug}");
        }
        for version in ["0.1.0", "1.0.0", "10.20.30"] {
            assert!(is_valid_version(version), "expected valid: {version}");
        }
        for version in ["", "1.0", "1.0.0.0", "v1.0.0", "1.0.x", "1..0"] {
            assert!(!is_valid_version(version), "expected invalid: {version}");
        }
    }

    #[test]
    fn init_scaffolds_valid_pack_input() {
        let base = tempfile::tempdir().unwrap();
        let dir = base.path().join("my-ability");
        let result = scaffold("my-ability", &dir).unwrap();
        assert_eq!(result.slug, "my-ability");
        assert!(dir.join("ability.json").is_file());
        assert!(dir.join("SKILL.md").is_file());
        assert!(dir.join("payload/.gitkeep").is_file());

        // The scaffold packs as-is, and its manifest fields parse into the
        // exact request types `ability publish` builds.
        let out = base.path().join("dist/my-ability");
        let packed = assemble(&dir, Some(out.clone())).unwrap();
        assert_eq!(packed.version, "0.1.0");
        let manifest: serde_json::Map<String, serde_json::Value> =
            serde_json::from_str(&fs::read_to_string(out.join("ability.json")).unwrap()).unwrap();
        manifest["slug"]
            .as_str()
            .unwrap()
            .parse::<PublishAbilityRequestSlug>()
            .unwrap();
        manifest["name"]
            .as_str()
            .unwrap()
            .parse::<PublishAbilityRequestName>()
            .unwrap();
        manifest["version"]
            .as_str()
            .unwrap()
            .parse::<PublishAbilityRequestVersion>()
            .unwrap();
        let visibility: AbilityVisibility =
            serde_json::from_value(manifest["visibility"].clone()).unwrap();
        assert_eq!(visibility, AbilityVisibility::Private);
        assert_eq!(manifest["min_tier"], serde_json::json!(""));
        assert_eq!(manifest["python_requirements"], serde_json::json!([]));
        // Scaffolding noise stays out of the package.
        assert!(!out.join(".gitkeep").exists());

        // Refuses bad slugs and existing directories.
        assert!(scaffold("Bad_Slug", &base.path().join("x")).is_err());
        assert!(scaffold("my-ability", &dir).is_err());
    }

    fn authoring_dir(base: &Path, manifest: serde_json::Value) -> PathBuf {
        let dir = base.join("src");
        fs::create_dir_all(dir.join("payload")).unwrap();
        fs::write(dir.join("ability.json"), manifest.to_string()).unwrap();
        fs::write(dir.join("SKILL.md"), "---\nname: my-ability\n---\n").unwrap();
        dir
    }

    fn manifest_json() -> serde_json::Value {
        serde_json::json!({
            "slug": "my-ability", "name": "My Ability", "version": "0.1.0",
            "description": "d", "required_env": [], "credit_cost_hint": "",
            "min_tier": "", "visibility": "private"
        })
    }

    #[test]
    fn pack_rejects_invalid_input() {
        let base = tempfile::tempdir().unwrap();
        let out_dir = base.path().join("out");
        let out = Some(out_dir.clone());

        let mut bad_slug = manifest_json();
        bad_slug["slug"] = "Bad_Slug".into();
        let dir = authoring_dir(&base.path().join("a"), bad_slug);
        let err = assemble(&dir, out.clone()).unwrap_err();
        assert!(err.to_string().contains("slug"), "{err}");

        let mut bad_version = manifest_json();
        bad_version["version"] = "1.0".into();
        let dir = authoring_dir(&base.path().join("b"), bad_version);
        let err = assemble(&dir, out.clone()).unwrap_err();
        assert!(err.to_string().contains("version"), "{err}");

        let mut bad_tier = manifest_json();
        bad_tier["min_tier"] = "platinum".into();
        let dir = authoring_dir(&base.path().join("c"), bad_tier);
        let err = assemble(&dir, out.clone()).unwrap_err();
        assert!(err.to_string().contains("min_tier"), "{err}");

        let mut bad_reqs = manifest_json();
        bad_reqs["python_requirements"] = serde_json::json!(["ok", 7]);
        let dir = authoring_dir(&base.path().join("d"), bad_reqs);
        let err = assemble(&dir, out.clone()).unwrap_err();
        assert!(err.to_string().contains("python_requirements"), "{err}");

        let dir = authoring_dir(&base.path().join("e"), manifest_json());
        fs::remove_file(dir.join("SKILL.md")).unwrap();
        let err = assemble(&dir, out.clone()).unwrap_err();
        assert!(err.to_string().contains("SKILL.md"), "{err}");

        let dir = authoring_dir(&base.path().join("f"), manifest_json());
        fs::write(
            dir.join("payload/big.bin"),
            vec![0u8; (MAX_PACKAGE_BYTES + 1) as usize],
        )
        .unwrap();
        let err = assemble(&dir, out).unwrap_err();
        assert!(err.to_string().contains("5 MiB"), "{err}");
        assert!(!out_dir.exists(), "oversize output must be cleaned up");
    }

    #[test]
    fn pack_output_matches_what_publish_consumes() {
        let base = tempfile::tempdir().unwrap();
        let dir = authoring_dir(base.path(), manifest_json());
        fs::write(dir.join("payload/tool.py"), "print('hi')\n").unwrap();
        fs::create_dir_all(dir.join("payload/pkg")).unwrap();
        fs::write(dir.join("payload/pkg/__init__.py"), "").unwrap();
        fs::create_dir_all(dir.join("payload/__pycache__")).unwrap();
        fs::write(dir.join("payload/__pycache__/junk.pyc"), "x").unwrap();

        let out = base.path().join("dist/my-ability");
        let packed = assemble(&dir, Some(out.clone())).unwrap();
        assert_eq!(packed.slug, "my-ability");
        assert_eq!(
            packed.files,
            vec!["SKILL.md", "ability.json", "pkg/__init__.py", "tool.py"]
        );

        // Payload contents sit at the package root, exactly where the
        // publish tarball + pod sync will put them next to SKILL.md.
        let targz = pack_targz(&out).unwrap();
        let root = base.path().join("skills");
        fs::create_dir_all(&root).unwrap();
        materialize(&root, "my-ability", "0.1.0", &targz).unwrap();
        let installed = root.join("my-ability");
        assert!(installed.join("ability.json").is_file());
        assert!(installed.join("SKILL.md").is_file());
        assert!(installed.join("tool.py").is_file());
        assert!(installed.join("pkg/__init__.py").is_file());
        assert!(!installed.join("payload").exists());
        assert!(!installed.join("__pycache__").exists());

        // A payload file that would shadow the manifest is refused.
        fs::write(dir.join("payload/ability.json"), "{}").unwrap();
        let err = assemble(&dir, Some(out)).unwrap_err();
        assert!(err.to_string().contains("payload/ability.json"), "{err}");
    }
}
