use anyhow::{Context, Result, bail, ensure};
use clap::{Args, Subcommand};
use nolgia_client::types::{
    CreateBlocksRenderRequest, CreateBlocksRenderRequestName, RenderAspectRatio, RenderBlock,
};
use uuid::Uuid;

use crate::output::{OutputFormat, print_json};

use super::compositions::{
    DEFAULT_POLL_INTERVAL_SECONDS, DEFAULT_RENDER_TIMEOUT_SECONDS, RenderOutcome, poll_render,
    report_render,
};
use super::{CommandContext, api_error};

#[derive(Subcommand, Debug)]
pub enum RenderCommand {
    /// Assemble ordered clip and narration pairs into a narrated explainer
    #[command(
        long_about = "Assemble ordered clip and narration pairs into a narrated explainer MP4.\n\nEach --pair is one fixed-length block, in the order given. A short narration\ntake is centered; a long take is sped up with pitch-preserving atempo to at\nmost 1.25x. A longer take is refused, naming the block and both asset ids.\nClips are trimmed from their start or have their last frame held to fill\nthe block. If an unknown duration is measured during rendering, an overlong\ntake fails the render with the same pair-specific reason.\n\nUse 1 to 60 pairs, 2 to 30 seconds per block, and at most 600 seconds total.\nRenders cost no credits. Use --wait to receive the finished asset URL."
    )]
    Blocks(BlocksArgs),
}

#[derive(Args, Debug)]
pub struct BlocksArgs {
    /// One video and narration pair per block; repeat in playback order
    #[arg(long = "pair", value_name = "VIDEO_ID:AUDIO_ID", required = true)]
    pub pairs: Vec<String>,
    /// Length of each block in seconds (2 to 30, fractions allowed)
    #[arg(long, value_name = "SECONDS", default_value_t = 10.0)]
    pub block_seconds: f64,
    /// Output canvas aspect ratio
    #[arg(long, value_name = "16:9|9:16|1:1", default_value = "16:9")]
    pub aspect: RenderAspectRatio,
    /// Keep each clip's own sound under the narration
    #[arg(long)]
    pub keep_video_audio: bool,
    /// Name for the carrier composition
    #[arg(long, value_name = "NAME")]
    pub name: Option<CreateBlocksRenderRequestName>,
    /// Project for the carrier composition and finished video
    #[arg(long, value_name = "UUID")]
    pub project: Option<Uuid>,
    /// Wait for the render to finish and resolve the produced asset
    #[arg(long)]
    pub wait: bool,
    /// Max seconds to wait for the render
    #[arg(long, value_name = "SECONDS", default_value_t = DEFAULT_RENDER_TIMEOUT_SECONDS)]
    pub timeout: u64,
    /// Seconds between render-status polls while waiting
    #[arg(long, value_name = "SECONDS", default_value_t = DEFAULT_POLL_INTERVAL_SECONDS)]
    pub poll_interval: u64,
}

pub async fn run(command: RenderCommand, ctx: &CommandContext) -> Result<()> {
    match command {
        RenderCommand::Blocks(args) => blocks(args, ctx).await,
    }
}

async fn blocks(args: BlocksArgs, ctx: &CommandContext) -> Result<()> {
    let n = args.pairs.len();
    ensure!((1..=60).contains(&n), "--pair requires 1 to 60 blocks");
    let seconds = args.block_seconds;
    ensure!(
        (2.0..=30.0).contains(&seconds),
        "--block-seconds must be between 2 and 30"
    );
    let duration = f64::from(u32::try_from(n)?) * seconds;
    ensure!(
        duration <= 600.0,
        "total duration must not exceed 600 seconds"
    );
    if args.wait {
        // Validate the wait knobs BEFORE submitting: poll_render checks them
        // too, but by then the render and its carrier composition exist and a
        // bare argument error would read like nothing happened.
        ensure!(args.timeout > 0, "--timeout must be greater than zero");
        ensure!(
            args.poll_interval > 0,
            "--poll-interval must be greater than zero"
        );
    }
    let mut blocks = Vec::with_capacity(n);
    for (index, pair) in args.pairs.iter().enumerate() {
        let parsed = pair.split_once(':').and_then(|(video, audio)| {
            Some((Uuid::parse_str(video).ok()?, Uuid::parse_str(audio).ok()?))
        });
        let Some((video_asset_id, audio_asset_id)) = parsed else {
            bail!(
                "--pair {} must be <video_asset_id>:<audio_asset_id> with two UUIDs",
                index + 1
            );
        };
        blocks.push(RenderBlock {
            video_asset_id,
            audio_asset_id,
        });
    }
    let body: CreateBlocksRenderRequest = CreateBlocksRenderRequest::builder()
        .blocks(blocks)
        .block_seconds(seconds)
        .aspect_ratio(args.aspect)
        .keep_video_audio(args.keep_video_audio)
        .name(args.name)
        .project_id(args.project)
        .try_into()
        .context("building blocks render request")?;
    let render = match ctx.client().create_blocks_render().body(body).send().await {
        Ok(response) => response.into_inner(),
        Err(err) => return Err(api_error(err, "submitting blocks render").await),
    };
    let render_id = render.id;
    eprintln!("render {render_id} submitted ({n} blocks, {seconds}s each)");

    if args.wait {
        let finished = poll_render(render_id, args.timeout, args.poll_interval, ctx).await?;
        return report_render(
            render.composition_id,
            RenderOutcome::Finished(Box::new(finished)),
            ctx,
        )
        .await;
    }
    match ctx.format() {
        OutputFormat::Json => print_json(&serde_json::json!({
            "render_id": render_id,
            "composition_id": render.composition_id,
            "status": "queued",
            "blocks": n,
            "block_seconds": seconds,
            "duration_seconds": duration,
        })),
        OutputFormat::Text => {
            println!("{render_id} queued");
            eprintln!("check it: nolgia compositions status {render_id}");
            Ok(())
        }
    }
}
