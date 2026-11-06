use anyhow::{Context, Result, bail, ensure};
use clap::{Args, Subcommand};
use nolgia_client::types::{
    CreateBlocksRenderRequest, CreateBlocksRenderRequestName, CreateGreenScreenRenderRequest,
    CreateGreenScreenRenderRequestMatchLevel, CreateGreenScreenRenderRequestName,
    CreateGreenScreenRenderRequestSubject, GreenScreenBackdrop, RenderAspectRatio, RenderBlock,
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
    /// Keep a person exactly as filmed: the grey stand-in, or the person back on top of a new world
    #[command(
        name = "keep-me",
        alias = "green-screen",
        long_about = "Keep a person exactly as filmed (POST /renders/green-screen). Free.\n\nOn real footage of a real person the person never goes through a video model:\n  1. cut them out: nolgia gen video --model remove-background-video --input <clip>\n  2. nolgia render keep-me --clip <clip> --backdrop other --matte <cut-out> --fill-subject\n     (the clip with the person filled flat grey: this is what the model sees)\n  3. the effect or new world on the grey clip: nolgia gen video --video-task edit --video-ref <grey clip> ...\n  4. nolgia render keep-me --clip <clip> --backdrop other --matte <cut-out> --background <world>\n     --background-from-clip --subject-filled --subject exact|match\n     (the person back on top from their own pixels; match moves only their brightness and colour)\n\nA clip shot on a green or blue screen needs no cut-out: --backdrop green|blue --background <picture or video>.\nAdd --matte <cut-out> to keep only the person: the screen is keyed inside the cut-out, so stands, walls and\nthe floor past the screen go too, and a clip with no screen is keyed from the cut-out alone."
    )]
    KeepMe(KeepMeArgs),
}

#[derive(Args, Debug)]
pub struct KeepMeArgs {
    /// The person's clip (the original footage), a video asset id
    #[arg(long, value_name = "UUID")]
    pub clip: Uuid,
    /// What they were filmed against: green, blue, white or other
    #[arg(long, value_name = "green|blue|white|other")]
    pub backdrop: Option<GreenScreenBackdrop>,
    /// The person's cut-out of this clip (remove-background-video): needed for white and other; with green or blue, only the person stays
    #[arg(long, value_name = "UUID")]
    pub matte: Option<Uuid>,
    /// Output the clip with the person filled flat grey (step 2); no background
    #[arg(long)]
    pub fill_subject: bool,
    /// The new background: a picture or video asset, or the world made from the grey clip
    #[arg(long, value_name = "UUID")]
    pub background: Option<Uuid>,
    /// The background video was made from this clip (laid frame for frame)
    #[arg(long)]
    pub background_from_clip: bool,
    /// The world was made from the grey-filled clip of step 2
    #[arg(long)]
    pub subject_filled: bool,
    /// The world already had the model's figure removed
    #[arg(long)]
    pub world_cleaned: bool,
    /// How the person goes back on top: exact (their own pixels) or match (brightness and colour only)
    #[arg(long, value_name = "exact|match")]
    pub subject: Option<CreateGreenScreenRenderRequestSubject>,
    /// How far match goes: subtle, medium or strong
    #[arg(long, value_name = "subtle|medium|strong")]
    pub match_level: Option<CreateGreenScreenRenderRequestMatchLevel>,
    /// Over a picture or video background: grade the person toward its light (their own pixels,
    /// brightness and colour only; the background keeps its look). Off: exactly as filmed
    #[arg(long = "match")]
    pub match_light: bool,
    /// Name for the carrier composition
    #[arg(long, value_name = "NAME")]
    pub name: Option<CreateGreenScreenRenderRequestName>,
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
        RenderCommand::KeepMe(args) => keep_me(args, ctx).await,
    }
}

/// The request `render keep-me` sends, checked before anything is submitted.
pub fn keep_me_request(args: &KeepMeArgs) -> Result<CreateGreenScreenRenderRequest> {
    if args.fill_subject {
        ensure!(
            args.background.is_none(),
            "--fill-subject makes the grey stand-in and takes no --background"
        );
        ensure!(
            args.matte.is_some(),
            "--fill-subject needs the person's cut-out: --matte <UUID>"
        );
    } else {
        ensure!(
            args.background.is_some(),
            "--background is required (or --fill-subject for the grey stand-in)"
        );
    }
    let flag = |on: bool| if on { Some(true) } else { None };
    CreateGreenScreenRenderRequest::builder()
        .asset_id(args.clip)
        .backdrop(args.backdrop)
        .matte_asset_id(args.matte)
        .fill_subject(flag(args.fill_subject))
        .background_asset_id(args.background)
        .background_from_clip(flag(args.background_from_clip))
        .subject_filled(flag(args.subject_filled))
        .world_cleaned(flag(args.world_cleaned))
        .subject(args.subject)
        .match_level(args.match_level)
        .match_(args.match_light)
        .name(args.name.clone())
        .project_id(args.project)
        .try_into()
        .context("building keep-me render request")
}

async fn keep_me(args: KeepMeArgs, ctx: &CommandContext) -> Result<()> {
    if args.wait {
        ensure!(args.timeout > 0, "--timeout must be greater than zero");
        ensure!(
            args.poll_interval > 0,
            "--poll-interval must be greater than zero"
        );
    }
    let body = keep_me_request(&args)?;
    let render = match ctx
        .client()
        .create_green_screen_render()
        .body(body)
        .send()
        .await
    {
        Ok(response) => response.into_inner(),
        Err(err) => return Err(api_error(err, "submitting keep-me render").await),
    };
    let render_id = render.id;
    eprintln!("render {render_id} submitted");
    crate::livejob::guard_render(render_id, async {
        if args.wait {
            let finished = poll_render(render_id, args.timeout, args.poll_interval, ctx).await?;
            return report_render(render.composition_id, RenderOutcome::Finished(Box::new(finished)), ctx).await;
        }
        match ctx.format() {
            OutputFormat::Json => print_json(
                ctx.output(),
                &serde_json::json!({"render_id": render_id, "composition_id": render.composition_id, "status": "queued"}),
            ),
            OutputFormat::Text => {
                println!("{render_id} queued");
                eprintln!("check it: nolgia compositions status {render_id}");
                Ok(())
            }
        }
    })
    .await
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

    crate::livejob::guard_render(render_id, async {
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
            OutputFormat::Json => print_json(
                ctx.output(),
                &serde_json::json!({
                    "render_id": render_id,
                    "composition_id": render.composition_id,
                    "status": "queued",
                    "blocks": n,
                    "block_seconds": seconds,
                    "duration_seconds": duration,
                }),
            ),
            OutputFormat::Text => {
                println!("{render_id} queued");
                eprintln!("check it: nolgia compositions status {render_id}");
                Ok(())
            }
        }
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args() -> KeepMeArgs {
        KeepMeArgs {
            clip: Uuid::nil(),
            backdrop: Some(GreenScreenBackdrop::Other),
            matte: Some(Uuid::from_u128(1)),
            fill_subject: false,
            background: Some(Uuid::from_u128(2)),
            background_from_clip: true,
            subject_filled: true,
            world_cleaned: false,
            subject: Some(CreateGreenScreenRenderRequestSubject::Match),
            match_level: Some(CreateGreenScreenRenderRequestMatchLevel::Subtle),
            match_light: false,
            name: None,
            project: None,
            wait: false,
            timeout: 1,
            poll_interval: 1,
        }
    }

    #[test]
    fn keep_me_sends_the_protect_fields() {
        let body = serde_json::to_value(keep_me_request(&args()).unwrap()).unwrap();
        assert_eq!(body["subject"], "match");
        assert_eq!(body["match_level"], "subtle");
        assert_eq!(body["background_from_clip"], true);
        assert_eq!(body["subject_filled"], true);
        assert_eq!(body["backdrop"], "other");
        // exactly as filmed unless --match: never graded by a default
        assert_eq!(body["match"], false);
        let mut lit = args();
        lit.match_light = true;
        let lit = serde_json::to_value(keep_me_request(&lit).unwrap()).unwrap();
        assert_eq!(lit["match"], true);
        assert!(
            body.get("fill_subject")
                .is_none_or(serde_json::Value::is_null)
        );
    }

    #[test]
    fn the_grey_stand_in_takes_a_cut_out_and_no_background() {
        let mut grey = args();
        grey.fill_subject = true;
        assert!(
            keep_me_request(&grey)
                .unwrap_err()
                .to_string()
                .contains("takes no --background")
        );
        grey.background = None;
        let body = serde_json::to_value(keep_me_request(&grey).unwrap()).unwrap();
        assert_eq!(body["fill_subject"], true);
        grey.matte = None;
        assert!(
            keep_me_request(&grey)
                .unwrap_err()
                .to_string()
                .contains("--matte")
        );
    }

    #[test]
    fn a_composite_needs_a_background() {
        let mut bare = args();
        bare.background = None;
        assert!(
            keep_me_request(&bare)
                .unwrap_err()
                .to_string()
                .contains("--background is required")
        );
    }
}
