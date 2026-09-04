//! Studio compositions: build a timeline from an ordered list of clips and
//! render it to one finished MP4.
//!
//! This is the "assemble and compile" surface the film/agent pipeline was
//! missing: `gen` produces individual clips and `projects` groups assets into
//! a folder, but neither sequences clips on a timeline or exports a single
//! cut. Without this, an agent asked to "assemble into a timeline and give me
//! the finished video" can only drop the clips into a project folder — a
//! collection, not a rendered video. `compositions create --render --wait`
//! closes that gap end to end: create a composition, author its `index.html`
//! timeline from the ordered clips, trigger a server render, and wait for the
//! finished asset.

use std::num::NonZeroU64;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail, ensure};
use base64::Engine;
use clap::{Args, Subcommand};
use nolgia_client::types::{
    CreateCompositionRequest, CreateRenderRequest, PutCompositionFileRequest,
};
use uuid::Uuid;

use crate::output::{OutputFormat, print_json};

use super::CommandContext;

/// Per-clip fallback when an asset has no recorded duration (e.g. a still).
const DEFAULT_CLIP_SECONDS: f64 = 5.0;
/// A server render is capped at 15 minutes; wait a touch longer by default.
const DEFAULT_RENDER_TIMEOUT_SECONDS: u64 = 900;
const DEFAULT_POLL_INTERVAL_SECONDS: u64 = 5;

#[derive(Subcommand, Debug)]
pub enum CompositionsCommand {
    /// Build a Studio composition (timeline) from an ordered list of clip
    /// assets, then optionally render it to one finished video
    Create(CreateArgs),
    /// Render an existing composition to a finished MP4 (optionally wait)
    Render(RenderArgs),
    /// Show one render's status (a single GET; no waiting)
    Status(StatusArgs),
    /// List your compositions, newest first
    List(ListArgs),
    /// Fetch one composition, including its file inventory
    Get(GetArgs),
}

#[derive(Args, Debug)]
pub struct CreateArgs {
    /// Composition name (unique per account; a duplicate returns 409)
    #[arg(long)]
    pub name: String,
    /// Link the composition to this project
    #[arg(long, value_name = "UUID")]
    pub project: Option<Uuid>,
    /// A clip asset to place on the timeline, in the order given (repeatable,
    /// at least one). Video, image, or audio assets you own.
    #[arg(long = "clip", value_name = "UUID", required = true)]
    pub clips: Vec<Uuid>,
    /// Output canvas width in pixels
    #[arg(long, default_value_t = 1920)]
    pub width: u32,
    /// Output canvas height in pixels
    #[arg(long, default_value_t = 1080)]
    pub height: u32,
    /// Strip the clips' embedded audio (silent output)
    #[arg(long)]
    pub mute: bool,
    /// Seconds to hold a clip whose asset has no recorded duration
    #[arg(long, value_name = "SECONDS", default_value_t = DEFAULT_CLIP_SECONDS)]
    pub default_duration: f64,
    /// Render the composition to a finished MP4 after building it
    #[arg(long)]
    pub render: bool,
    /// Wait for the render to finish and resolve the produced asset (implies --render)
    #[arg(long)]
    pub wait: bool,
    /// Max seconds to wait for the render before giving up the wait
    #[arg(long, default_value_t = DEFAULT_RENDER_TIMEOUT_SECONDS)]
    pub timeout: u64,
    /// Seconds between render-status polls while waiting
    #[arg(long, value_name = "SECONDS", default_value_t = DEFAULT_POLL_INTERVAL_SECONDS)]
    pub poll_interval: u64,
}

#[derive(Args, Debug)]
pub struct RenderArgs {
    /// Composition to render
    pub composition_id: Uuid,
    /// Wait for the render to finish and resolve the produced asset
    #[arg(long)]
    pub wait: bool,
    #[arg(long, default_value_t = DEFAULT_RENDER_TIMEOUT_SECONDS)]
    pub timeout: u64,
    #[arg(long, value_name = "SECONDS", default_value_t = DEFAULT_POLL_INTERVAL_SECONDS)]
    pub poll_interval: u64,
}

#[derive(Args, Debug)]
pub struct StatusArgs {
    /// Render id (from `compositions render`/`create --render`)
    pub render_id: Uuid,
}

#[derive(Args, Debug)]
pub struct ListArgs {
    /// Only compositions linked to this project
    #[arg(long, value_name = "UUID")]
    pub project: Option<Uuid>,
}

#[derive(Args, Debug)]
pub struct GetArgs {
    pub composition_id: Uuid,
}

pub async fn run(command: CompositionsCommand, ctx: &CommandContext) -> Result<()> {
    match command {
        CompositionsCommand::Create(args) => create(args, ctx).await,
        CompositionsCommand::Render(args) => render(args, ctx).await,
        CompositionsCommand::Status(args) => status(args, ctx).await,
        CompositionsCommand::List(args) => list(args, ctx).await,
        CompositionsCommand::Get(args) => get(args, ctx).await,
    }
}

/// One resolved timeline clip: its media reference plus the timing the
/// authored HTML will carry.
struct Clip {
    tag: &'static str,
    asset_id: Uuid,
    duration: f64,
}

/// Author a minimal, renderer-honored `index.html` timeline: one media element
/// per clip, back to back on a single track, referencing platform assets by
/// the `asset:<uuid>` scheme so no media bytes need bundling. The renderer
/// windows every `[data-start][data-duration]` element and mixes each media
/// element's audio into the master track unless `muted`.
fn author_index_html(clips: &[Clip], width: u32, height: u32, mute: bool) -> String {
    let mut out = String::new();
    out.push_str("<!DOCTYPE html>\n<html>\n<head><style>\n");
    out.push_str("  html, body { margin: 0; background: #000; }\n");
    out.push_str("  body { position: relative; }\n");
    out.push_str("  video, img { position: absolute; inset: 0; width: 100%; height: 100%; object-fit: cover; }\n");
    out.push_str("</style></head>\n");
    out.push_str(&format!(
        "<body data-width=\"{width}\" data-height=\"{height}\">\n"
    ));
    let mut start = 0.0_f64;
    let muted_attr = if mute { " muted" } else { "" };
    for (i, clip) in clips.iter().enumerate() {
        let n = i + 1;
        // Images/audio ignore the muted attr harmlessly; only video honors it.
        let this_muted = if clip.tag == "video" { muted_attr } else { "" };
        out.push_str(&format!(
            "  <{tag} id=\"clip{n}\" src=\"asset:{asset}\" preload=\"auto\" playsinline{muted} \
data-start=\"{start}\" data-duration=\"{dur}\" data-track-index=\"0\"></{tag}>\n",
            tag = clip.tag,
            asset = clip.asset_id,
            muted = this_muted,
            start = trim_f64(start),
            dur = trim_f64(clip.duration),
        ));
        start += clip.duration;
    }
    out.push_str("</body>\n</html>\n");
    out
}

/// Format a float without a trailing `.0` so `5.0` becomes `5` (matches the
/// seconds the web editor snaps to and keeps the HTML tidy).
fn trim_f64(v: f64) -> String {
    let s = format!("{v:.3}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    s.to_string()
}

async fn create(args: CreateArgs, ctx: &CommandContext) -> Result<()> {
    ensure!(
        args.width >= 16 && args.height >= 16,
        "--width and --height must be at least 16"
    );
    ensure!(
        args.default_duration > 0.0,
        "--default-duration must be greater than zero"
    );

    // Resolve each clip's media kind + duration from its asset metadata, in
    // the order given. A still (no duration) holds for --default-duration.
    let mut clips = Vec::with_capacity(args.clips.len());
    for asset_id in &args.clips {
        let asset = ctx
            .client()
            .get_asset()
            .id(*asset_id)
            .send()
            .await
            .with_context(|| format!("fetching clip asset {asset_id}"))?
            .into_inner();
        let modality = asset.modality.to_string();
        let tag = match modality.as_str() {
            "image" => "img",
            "audio" => "audio",
            _ => "video",
        };
        let duration = asset
            .duration_seconds
            .filter(|d| *d > 0.0)
            .unwrap_or(args.default_duration);
        clips.push(Clip {
            tag,
            asset_id: *asset_id,
            duration,
        });
    }

    let html = author_index_html(&clips, args.width, args.height, args.mute);

    // Create the empty composition, then upload its index.html timeline.
    let body: CreateCompositionRequest = CreateCompositionRequest::builder()
        .name(args.name)
        .project_id(args.project)
        .try_into()
        .context("building create-composition request")?;
    let composition = ctx
        .client()
        .create_composition()
        .body(body)
        .send()
        .await
        .context("creating composition")?
        .into_inner();
    let composition_id = composition.id;

    let file_body: PutCompositionFileRequest = PutCompositionFileRequest::builder()
        .content_base64(base64::engine::general_purpose::STANDARD.encode(html.as_bytes()))
        .content_type("text/html")
        .try_into()
        .context("building index.html upload")?;
    ctx.client()
        .put_composition_file()
        .id(composition_id)
        .path("index.html")
        .body(file_body)
        .send()
        .await
        .context("uploading index.html timeline")?;

    // Stop here unless a render was asked for.
    if !args.render && !args.wait {
        return match ctx.format() {
            OutputFormat::Json => print_json(&serde_json::json!({
                "composition_id": composition_id,
                "clips": clips.len(),
            })),
            OutputFormat::Text => {
                println!("{composition_id} ({} clip(s) on the timeline)", clips.len());
                eprintln!("edit in Studio: https://nolgia.ai/studio/{composition_id}");
                eprintln!("render it: nolgia compositions render {composition_id} --wait");
                Ok(())
            }
        };
    }

    let outcome = trigger_render(
        composition_id,
        args.wait,
        args.timeout,
        args.poll_interval,
        ctx,
    )
    .await?;
    report_render(composition_id, outcome, ctx).await
}

async fn render(args: RenderArgs, ctx: &CommandContext) -> Result<()> {
    let outcome = trigger_render(
        args.composition_id,
        args.wait,
        args.timeout,
        args.poll_interval,
        ctx,
    )
    .await?;
    report_render(args.composition_id, outcome, ctx).await
}

/// What a render invocation resolved to.
enum RenderOutcome {
    /// Submitted, not waited for.
    Submitted { render_id: Uuid },
    /// Waited to completion; carries the finished render row.
    Finished(Box<nolgia_client::types::Render>),
}

async fn trigger_render(
    composition_id: Uuid,
    wait: bool,
    timeout: u64,
    poll_interval: u64,
    ctx: &CommandContext,
) -> Result<RenderOutcome> {
    // Always send a body: a bodyless POST is rejected by the production load
    // balancer with 411 (same reason `finish_asset_upload` exists). An empty
    // `params` is the reserved-but-empty body the endpoint accepts.
    let body: CreateRenderRequest = CreateRenderRequest::builder()
        .try_into()
        .context("building render request")?;
    let render = ctx
        .client()
        .create_composition_render()
        .id(composition_id)
        .body(body)
        .send()
        .await
        .context("submitting composition render")?
        .into_inner();
    let render_id = render.id;
    // Surface the id immediately on stderr so a later wait timeout or Ctrl-C
    // never loses the handle to in-flight (Cloud Run) render work.
    eprintln!("render {render_id} submitted for composition {composition_id}");

    if !wait {
        return Ok(RenderOutcome::Submitted { render_id });
    }
    let finished = poll_render(render_id, timeout, poll_interval, ctx).await?;
    Ok(RenderOutcome::Finished(Box::new(finished)))
}

/// Poll `GET /renders/{id}` until the render reaches a terminal state. Renders
/// have no server-side long-poll (unlike jobs), so this is a client-side loop.
async fn poll_render(
    render_id: Uuid,
    timeout: u64,
    poll_interval: u64,
    ctx: &CommandContext,
) -> Result<nolgia_client::types::Render> {
    let _ = NonZeroU64::new(timeout).context("--timeout must be greater than zero")?;
    let interval =
        NonZeroU64::new(poll_interval).context("--poll-interval must be greater than zero")?;
    let deadline = Instant::now() + Duration::from_secs(timeout);
    loop {
        let render = ctx
            .client()
            .get_render()
            .id(render_id)
            .send()
            .await
            .context("polling render status")?
            .into_inner();
        match render.status.to_string().as_str() {
            "succeeded" => return Ok(render),
            "failed" => bail!(
                "render {render_id} failed: {}",
                render.error.as_deref().unwrap_or("no reason given")
            ),
            _ => {}
        }
        if Instant::now() >= deadline {
            bail!(
                "render {render_id} still processing after {timeout}s; \
                 it keeps going server-side, check it with `nolgia compositions status {render_id}`"
            );
        }
        tokio::time::sleep(Duration::from_secs(interval.get())).await;
    }
}

async fn report_render(
    composition_id: Uuid,
    outcome: RenderOutcome,
    ctx: &CommandContext,
) -> Result<()> {
    match outcome {
        RenderOutcome::Submitted { render_id } => match ctx.format() {
            OutputFormat::Json => print_json(&serde_json::json!({
                "composition_id": composition_id,
                "render_id": render_id,
                "status": "queued",
            })),
            OutputFormat::Text => {
                println!("{render_id} queued");
                Ok(())
            }
        },
        RenderOutcome::Finished(render) => {
            let asset_id = render
                .asset_id
                .context("render succeeded but produced no asset")?;
            let asset = ctx
                .client()
                .get_asset()
                .id(asset_id)
                .send()
                .await
                .context("fetching rendered asset")?
                .into_inner();
            if !render.warnings.is_empty() {
                for w in &render.warnings {
                    eprintln!("render warning: {w}");
                }
            }
            match ctx.format() {
                OutputFormat::Json => print_json(&serde_json::json!({
                    "composition_id": composition_id,
                    "render_id": render.id,
                    "asset_id": asset_id,
                    "status": render.status.to_string(),
                    "url": asset.signed_url,
                    "warnings": render.warnings,
                })),
                OutputFormat::Text => {
                    println!("{}", asset.signed_url);
                    Ok(())
                }
            }
        }
    }
}

async fn status(args: StatusArgs, ctx: &CommandContext) -> Result<()> {
    let render = ctx
        .client()
        .get_render()
        .id(args.render_id)
        .send()
        .await
        .context("fetching render status")?
        .into_inner();
    match ctx.format() {
        OutputFormat::Json => print_json(&render),
        OutputFormat::Text => {
            print!("{} {}", render.id, render.status);
            match (render.asset_id, render.error.as_deref()) {
                (Some(asset), _) => println!(" asset={asset}"),
                (None, Some(err)) => println!(" error={err}"),
                (None, None) => println!(),
            }
            Ok(())
        }
    }
}

async fn list(args: ListArgs, ctx: &CommandContext) -> Result<()> {
    let mut builder = ctx.client().list_compositions();
    if let Some(project) = args.project {
        builder = builder.project_id(project);
    }
    let list = builder
        .send()
        .await
        .context("listing compositions")?
        .into_inner();
    match ctx.format() {
        OutputFormat::Json => print_json(&list),
        OutputFormat::Text => {
            for composition in list.compositions {
                println!("{} {}", composition.id, composition.name.as_str());
            }
            Ok(())
        }
    }
}

async fn get(args: GetArgs, ctx: &CommandContext) -> Result<()> {
    let composition = ctx
        .client()
        .get_composition()
        .id(args.composition_id)
        .send()
        .await
        .context("fetching composition")?
        .into_inner();
    match ctx.format() {
        OutputFormat::Json => print_json(&composition),
        OutputFormat::Text => {
            println!("{} {}", composition.id, composition.name.as_str());
            Ok(())
        }
    }
}
