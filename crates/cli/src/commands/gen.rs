use anyhow::{Context, Result};
use clap::{Args, Subcommand};
use nolgia_client::ClientExt;
use nolgia_client::types::{
    AspectRatio, AudioFormat, BitrateMode, CameraMoveStrength, CreateAssetUploadRequest,
    CreateAssetUploadRequestContentType, GenerateAudioRequest, GenerateImageRequest,
    GenerateImageRequestQuality, GenerateVideoRequest, GenerateVideoRequestMotionId,
    GenerateVideoRequestNegativePrompt, GenerateVideoRequestQuality, ImageAspectRatio,
    UploadAssetRequest, UploadAssetRequestContentType, UploadAssetRequestFilename, VideoShot,
};
use serde::Serialize;
use std::{
    fs,
    path::{Path, PathBuf},
};

use crate::livejob;
use crate::output::{OutputFormat, print_json};

use super::CommandContext;

#[derive(Subcommand, Debug)]
pub enum GenCommand {
    Image(ImageArgs),
    Video(VideoArgs),
    Audio(AudioArgs),
    /// Turn one to four photos into a 3D model (GLB)
    #[command(name = "3d")]
    ThreeD(ThreeDArgs),
}

#[derive(Args, Debug)]
pub struct ImageArgs {
    /// Model id (see `nolgia models list --modality image`). Any id the API
    /// accepts is forwarded verbatim, so a model added after this binary was
    /// built still works — the API is the authority on what exists.
    #[arg(long, default_value = "flux-pro")]
    pub model: String,
    /// What to draw. Required, except on a model that takes no prompt: one
    /// marked `remove background` (cuts the subject out onto a transparent
    /// PNG) or `enhance` (an upscaler) in `nolgia models list`, which works
    /// on --input alone and refuses a prompt. Optional with --expand-to.
    #[arg(long)]
    pub prompt: Option<String>,
    /// The image to edit: a local file (uploaded to /assets) or the UUID of
    /// an existing asset. Rides as `reference_asset_ids`, so the server
    /// re-signs it at execution time and a queued job never runs with an
    /// expired URL. Needs a model that accepts reference images
    /// (`reference images` in `nolgia models get <model>`).
    #[arg(long, value_name = "PATH_OR_UUID")]
    pub input: Option<String>,
    /// Edit only PART of --input: a PNG with an alpha channel, the same pixel
    /// dimensions as the image, whose TRANSPARENT areas are the region the
    /// model may repaint. Everything the mask leaves opaque is preserved.
    /// Takes a local file (uploaded to /assets) or an asset UUID.
    ///
    /// Only on models whose catalog entry publishes `inpaint mask`
    /// (`nolgia models get <model>`) — the GPT Image family — and only
    /// alongside exactly one --input. A masked edit costs exactly what the
    /// same model's ordinary generation costs; a mask changes no price.
    #[arg(long, value_name = "PATH_OR_UUID", requires = "input")]
    pub mask: Option<String>,
    /// How much detail the model spends DRAWING the image, on models that
    /// publish a render-quality ladder (`nolgia models get <model>`).
    ///
    /// This is a SECOND axis and not a rename of --quality: --quality is the
    /// native/2k/4k UPSCALE ladder, which re-renders the finished image
    /// larger, while this changes how the picture is drawn in the first place.
    /// They compose, and a request may carry both.
    ///
    /// `auto` (the default) is the model's own choice at the base rate.
    /// `low`/`medium`/`high` spend LESS and cost the same, so they buy speed,
    /// not savings. `xhigh` and `max` spend substantially more and ADD credits
    /// PER IMAGE — the exact figure is in `nolgia models get <model>`, and the
    /// CLI prints it before submitting. They exist only on the GPT Image 2.5
    /// models.
    #[arg(long, value_name = "VALUE")]
    pub render_quality: Option<String>,
    #[arg(long)]
    pub out: Option<PathBuf>,
    /// Quality/resolution tier (model-specific; tiers and per-tier credits
    /// in `nolgia models get <model>`). Omit for the model's default tier.
    #[arg(long)]
    pub quality: Option<String>,
    /// Output aspect ratio, e.g. 16:9, 9:16, 1:1, 4:3, 3:4 (model-dependent).
    /// The values a given model accepts are listed as `aspect ratios` in
    /// `nolgia models get <model>`. Omit for the model's native default.
    #[arg(long, value_parser = parse_image_aspect_ratio)]
    pub aspect_ratio: Option<ImageAspectRatio>,
    /// Outpaint --input to RATIO by painting new content into the added margins,
    /// keeping the source pixels (e.g. a 16:9 still to 9:16). Uses flux-expand
    /// unless --model names another model that publishes image_expand.
    /// --prompt becomes optional and describes the new area. Flat price per
    /// image (see `nolgia models get flux-expand`).
    #[arg(long, value_name = "RATIO", value_parser = parse_image_aspect_ratio, requires = "input", conflicts_with_all = ["aspect_ratio", "quality", "character_id", "face_reference_asset_id", "mask", "render_quality"])]
    pub expand_to: Option<ImageAspectRatio>,
    /// Apply Aura, the Nolgia character engine: server-side photoreal
    /// composition layered onto the prompt (people render as photographs,
    /// no AI gloss). Honored only on models whose catalog entry publishes
    /// `aura compatible` (`nolgia models get <model>`); `true` on any other
    /// model is a no-op, never an error. Omit the flag for the server's
    /// default: ON for prompts that read as a person subject on compatible
    /// models, OFF otherwise. An explicit `false` always wins.
    #[arg(long, action = clap::ArgAction::Set)]
    pub aura: Option<bool>,
    /// Image asset (one of yours) whose face conditions the render for
    /// identity. A face reference IS an Aura identity request: it turns the
    /// pipeline on (`--aura false` alongside it is refused) and needs a
    /// model that accepts reference images. The delivered render is subject
    /// to the ArcFace identity gate (>= 0.60 against this reference, one
    /// automatic re-roll) once the scoring runtime is enabled.
    #[arg(long, value_name = "ASSET_UUID")]
    pub face_reference_asset_id: Option<uuid::Uuid>,
    /// Render one of your characters (`nolgia characters list`): its primary
    /// reference becomes this render's face reference and its canonical
    /// description rides into the prompt verbatim, so the same character
    /// renders consistently across generations. Inherits every
    /// --face-reference-asset-id rule; the two flags cannot be combined
    /// (two competing identities are refused, not resolved).
    #[arg(
        long,
        value_name = "CHARACTER_UUID",
        conflicts_with = "face_reference_asset_id"
    )]
    pub character_id: Option<uuid::Uuid>,
    /// File the generated asset(s) into this project (`nolgia projects
    /// list` for ids). The project must exist and belong to you.
    #[arg(long, value_name = "PROJECT_UUID")]
    pub project_id: Option<uuid::Uuid>,
    #[arg(long, default_value_t = false)]
    pub wait: bool,
    #[arg(long, default_value_t = false)]
    pub no_wait: bool,
}

#[derive(Args, Debug)]
#[command(after_help = "Video jobs cost credits (see `nolgia models list`). \
Agents: estimate with --cost-only first and confirm with the user before \
submitting batches over ~2000 credits.")]
pub struct VideoArgs {
    /// Model id (see `nolgia models list --modality video`). Any id the API
    /// accepts is forwarded verbatim and validated server-side, so a model the
    /// API already serves works even on a binary built before it was added
    /// (NOL-439: `flux-3-video` was rejected by the closed client-side enum
    /// though the API accepted it). The API is the authority on what exists.
    /// The default is `seedance-2.5`, matching the API's own
    /// DefaultVideoModel. It was `fal-ai/kling-video/v3/text-to-video` until
    /// 2026-09-09, when the founder hid Kling indefinitely after the prepaid
    /// provider account ran dry: `nolgia gen video` with no `--model` was the
    /// highest-impact silent route to that dead account anywhere in the
    /// platform, because nobody had to type a Kling id to reach it.
    #[arg(long, default_value = "seedance-2.5")]
    pub model: String,
    /// What to generate. Required, except on a background-removal model
    /// (`remove background` in `nolgia models list`), which takes no prompt.
    #[arg(long)]
    pub prompt: Option<String>,
    /// Start image: a local file (uploaded to /assets) or the UUID of an
    /// existing asset (reused, fresh signed URL). Required for
    /// image-to-video models; optional on models with image input
    /// support (Veo, Omni Flash) per `nolgia models list`.
    ///
    /// On a background-removal model (`--model remove-background-video`)
    /// --input is instead the CLIP to cut the subject out of, and the only
    /// input: one of your video asset UUIDs, a local video file (uploaded to
    /// /assets first) or an https URL (which needs --duration-seconds). No
    /// prompt. It is billed on the clip's length rounded up to whole seconds
    /// (`nolgia models get remove-background-video`), and the result is a
    /// WebM (VP9) with an alpha channel, so --out saves it as .webm.
    #[arg(long)]
    pub input: Option<String>,
    #[arg(long)]
    pub out: Option<PathBuf>,
    /// e.g. 16:9, 9:16, 1:1, 4:3, 3:4 (model-dependent)
    #[arg(long)]
    pub aspect_ratio: Option<AspectRatio>,
    /// Clip length in seconds (model-dependent; Kling/Seedance 3-15, Veo 4/6/8,
    /// Omni Flash 3-10). Omit to let the server choose: 5s normally, or the sum
    /// of the --shot durations when shots are given. If passed alongside --shot
    /// it must equal that sum.
    ///
    /// With ONE --video-ref on a model whose reference video shows the
    /// subject to perform (Seedance reference-to-video), omitting it renders
    /// the reference's length instead: its stored duration counted to the
    /// nearest whole second, the way the API counts a reference (a 5.25s
    /// clip renders 5s, a 9.6s clip 10s), fitted to the model's range. The
    /// CLI prints the length it chose. With several --video-ref clips, or a
    /// clip whose length is not stored, the server default applies; pass the
    /// length you want.
    #[arg(long)]
    pub duration_seconds: Option<std::num::NonZeroU64>,
    #[arg(long)]
    pub seed: Option<u64>,
    #[arg(long)]
    pub negative_prompt: Option<String>,
    // Keep this description capability-driven and free of model names: the
    // enumeration it replaced ("Seedance/Veo") was a second source of truth
    // that silently rotted (NOL-352). Pinned by the
    // audio_flag_help_stays_capability_driven test.
    //
    // The behaviour below is published per model as `video.audio` on
    // `GET /models` (nolgia-api#224) and, since the re-vendor in #83, is in
    // the spec this crate generates from — so `--json` now carries it. This
    // text still does not send the reader to `nolgia models list` for it,
    // because `capability_line` does not render the field yet, and citing a
    // capability the reader cannot see is the same broken promise in a new
    // place. Teach `models list` to show it, then cite it here by name.
    /// Generate a synchronized audio track. What this achieves is set by the
    /// model, not by the flag: models without audio render silent whatever
    /// you pass, models whose audio is native always produce it (so
    /// `--generate-audio false` is rejected), and the rest honor the flag.
    /// Omit it to get audio wherever the model can be asked for it.
    #[arg(long, action = clap::ArgAction::Set)]
    pub generate_audio: Option<bool>,
    /// Quality/resolution tier, e.g. 720p/1080p/4k on Seedance 2.0 Pro.
    /// Model-specific; tiers and per-tier credits in `nolgia models get
    /// <model>` (premium tiers cost more). Omit for the default tier.
    #[arg(long)]
    pub quality: Option<String>,
    /// Output bitrate profile (standard|high) on models with a bitrate
    /// knob (`nolgia models get <model>`)
    #[arg(long)]
    pub bitrate: Option<BitrateMode>,
    /// Camera move from the library (`nolgia motions list`), e.g. push-in,
    /// orbit-left, crane-up, rack-focus. The server appends the move's
    /// prompt fragment to --prompt; it never replaces your prompt. Works on
    /// every video model, with or without --input.
    #[arg(long, value_name = "MOTION_ID")]
    pub motion: Option<String>,
    /// How far and how fast the camera move travels: subtle, medium (the
    /// default) or strong. Needs --motion.
    #[arg(long, value_name = "STRENGTH", requires = "motion")]
    pub motion_strength: Option<CameraMoveStrength>,
    /// Reference video for reference-to-video models: the UUID of one of
    /// your video assets, repeated up to the model's `video_refs_max` in
    /// `nolgia models get <model> --json` (10 on seedance-2.5, 3 on Seedance
    /// 2.0 Pro). Address them in the prompt as @Video1, @Video2 and so on.
    /// Inputs: MP4/MOV and 50MB combined; seedance-2.5 takes 2-30s combined,
    /// Seedance 2.0 Pro 2-15s at 480p-720p. See --duration-seconds for how
    /// one reference sets the clip length.
    #[arg(long = "video-ref", value_name = "ASSET_ID")]
    pub video_refs: Vec<uuid::Uuid>,
    /// Element/reference image for reference-to-video models: the UUID of
    /// one of your image assets (repeat up to 9). Address them in the
    /// prompt as @Image1..@Image9.
    #[arg(long = "element", value_name = "ASSET_ID")]
    pub elements: Vec<uuid::Uuid>,
    /// Reference audio track: an audio asset UUID or a local file (uploaded
    /// to /assets first). Repeatable, up to the model's `audio refs` in
    /// `nolgia models get <model>`.
    ///
    /// This is how lip sync is reached: `heygen-avatar-iv` takes one portrait
    /// (--input) plus the voice track it speaks, and the clip is BILLED on
    /// that track's stored duration — which is why the API takes an asset id
    /// and refuses a raw URL, and why --duration-seconds should be left off
    /// on such a model (the length comes from the audio).
    #[arg(long = "audio-ref", value_name = "PATH_OR_UUID")]
    pub audio_refs: Vec<String>,
    /// Final frame for start+end frame pinning (models with end-frame
    /// support): an image asset UUID or a local file (uploaded). Requires
    /// --input (the start frame).
    #[arg(long = "end-frame", value_name = "ASSET_ID")]
    pub end_frame: Option<String>,
    /// Print the credit estimate from the live catalog and exit without
    /// creating a job
    #[arg(long, default_value_t = false)]
    pub cost_only: bool,
    /// Multi-shot segment "SECONDS:PROMPT" or "SECONDS:PROMPT|AUDIO DIRECTION".
    /// Repeat up to 8 times; clip duration = sum, --prompt becomes style/context.
    #[arg(long = "shot")]
    pub shots: Vec<String>,
    /// Bind the clip to one of your characters (`nolgia characters list`):
    /// its primary reference is attached as an element reference (taking
    /// the next @Image slot after any --element images) and its canonical
    /// description rides into the prompt verbatim, keeping the character
    /// consistent between a still and a clip. Needs a model with room for
    /// one more element reference (`nolgia models get <model>`).
    #[arg(long, value_name = "CHARACTER_UUID")]
    pub character_id: Option<uuid::Uuid>,
    /// File the generated asset into this project (`nolgia projects list`
    /// for ids). The project must exist and belong to you.
    #[arg(long, value_name = "PROJECT_UUID")]
    pub project_id: Option<uuid::Uuid>,
    #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
    pub wait: bool,
    #[arg(long, default_value_t = false)]
    pub no_wait: bool,
    #[arg(long, default_value_t = 300)]
    pub timeout: u64,
}

#[derive(Args, Debug)]
pub struct AudioArgs {
    /// Model id (see `nolgia models list --modality audio`). Any id the API
    /// accepts is forwarded verbatim, so a model added after this binary was
    /// built still works — the API is the authority on what exists.
    #[arg(long, default_value = "fal-ai/stable-audio-25/text-to-audio")]
    pub model: String,
    #[arg(long)]
    pub prompt: String,
    #[arg(long)]
    pub input: Option<PathBuf>,
    #[arg(long)]
    pub out: Option<PathBuf>,
    /// Voice id for TTS models (see `nolgia voices list --model <model>`)
    #[arg(long)]
    pub voice: Option<String>,
    #[arg(long, default_value = "mp3")]
    pub format: AudioFormat,
    /// File the generated asset into this project (`nolgia projects list`
    /// for ids). The project must exist and belong to you.
    #[arg(long, value_name = "PROJECT_UUID")]
    pub project_id: Option<uuid::Uuid>,
    #[arg(long, default_value_t = false)]
    pub wait: bool,
    #[arg(long, default_value_t = false)]
    pub no_wait: bool,
}

/// Every value the API's `ImageAspectRatio` enum accepts, in spec order.
///
/// Only used to render a useful parse error — the authoritative check is
/// per-model against `image.aspect_ratios` from `GET /models`. Kept honest by
/// `image_aspect_ratio_choices_match_the_spec`, which reads the vendored
/// OpenAPI spec and fails if this list ever drifts from the real enum.
pub const IMAGE_ASPECT_RATIOS: &[&str] = &[
    "16:9", "9:16", "1:1", "4:3", "3:4", "3:2", "2:3", "21:9", "9:21", "2:1", "1:2", "5:4", "4:5",
    "3:1", "1:3", "4:1", "1:4", "8:1", "1:8",
];

/// Parse `--aspect-ratio`, naming every accepted value on a miss.
///
/// The generated enum's own `FromStr` error is the bare string "invalid
/// value", which tells the caller nothing — and the values people reach for
/// first are the `image_size` aliases (`portrait_16_9`, and NOL-331's
/// `portrait_1080_1920`), which are a different vocabulary entirely.
fn parse_image_aspect_ratio(raw: &str) -> Result<ImageAspectRatio, String> {
    ImageAspectRatio::try_from(raw).map_err(|_| {
        format!(
            "expected a ratio, one of: {}. (Note these are ratios, not \
             `image_size` aliases like `portrait_16_9`.)",
            IMAGE_ASPECT_RATIOS.join(", ")
        )
    })
}

#[derive(Serialize)]
pub(crate) struct AsyncJob {
    pub(crate) job_id: String,
}

const DEFAULT_WAIT_TIMEOUT_SECONDS: u64 = 300;

pub async fn run(command: GenCommand, ctx: &CommandContext) -> Result<()> {
    match command {
        GenCommand::Image(args) => image(args, ctx).await,
        GenCommand::Video(args) => video(args, ctx).await,
        GenCommand::Audio(args) => audio(args, ctx).await,
        GenCommand::ThreeD(args) => three_d(args, ctx).await,
    }
}

async fn image(args: ImageArgs, ctx: &CommandContext) -> Result<()> {
    let model = if args.expand_to.is_some() && args.model == "flux-pro" {
        "flux-expand".to_string()
    } else {
        args.model
    };
    // Only a missing prompt costs a catalog read: a prompt on a model that
    // takes none is refused by the server in the same words.
    if args.prompt.is_none() && args.expand_to.is_none() {
        let entry = super::models::entry(ctx, &model).await;
        check_promptless_image(&model, entry.as_ref(), args.input.is_some())?;
    }
    if args.expand_to.is_some() {
        super::models::precheck_image_expand(ctx, &model).await?;
    }
    let aspect_ratio = args.expand_to.or(args.aspect_ratio);
    if let Some(tier) = args.quality.as_deref() {
        super::models::precheck_image_quality(ctx, &model, tier).await?;
    }
    if let Some(ratio) = aspect_ratio.as_ref() {
        super::models::precheck_image_aspect_ratio(ctx, &model, ratio).await?;
    }
    if args.mask.is_some() {
        super::models::precheck_inpaint_mask(ctx, &model).await?;
    }
    // The render-quality adder is announced BEFORE anything is uploaded or
    // submitted. It is per IMAGE, and that is the part that surprises people:
    // four images at max pay it four times.
    let render_quality = args
        .render_quality
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty());
    if let Some(value) = render_quality {
        let added = super::models::precheck_render_quality(ctx, &model, value).await?;
        if added > 0 {
            eprintln!(
                "render quality {value}: +{added} credits per image, on top of whatever \
                 --quality costs"
            );
        }
    }
    // Everything the mask has to satisfy is checked BEFORE either file is
    // uploaded: a refusal should cost neither an upload nor a round trip.
    let mask_bytes = match args.mask.as_deref() {
        Some(mask) if Path::new(mask).exists() => Some(read_mask_png(Path::new(mask))?),
        _ => None,
    };
    if let Some(mask) = &mask_bytes
        && let Some(input) = args.input.as_deref()
        && Path::new(input).exists()
    {
        let image = png_dimensions(&fs::read(input).with_context(|| format!("reading {input}"))?);
        if let Some((width, height)) = image
            && (width, height) != (mask.width, mask.height)
        {
            anyhow::bail!(
                "--mask is {}x{} but --input is {width}x{height} — a mask must match its image \
                 exactly, because the transparent pixels ARE the region to repaint. Re-export \
                 the mask at the image's size.",
                mask.width,
                mask.height
            );
        }
    }

    let reference_asset_ids = match args.input.as_deref() {
        Some(input) => vec![resolve_reference_asset(input, "--input", ctx).await?],
        None => Vec::new(),
    };
    let mask_asset_id = match args.mask.as_deref() {
        Some(mask) => Some(resolve_reference_asset(mask, "--mask", ctx).await?),
        None => None,
    };

    let quality = args
        .quality
        .as_deref()
        .map(GenerateImageRequestQuality::try_from)
        .transpose()
        .map_err(|e| anyhow::anyhow!("--quality: {e}"))?;
    let prompt = args
        .prompt
        .map(nolgia_client::types::GenerateImageRequestPrompt::try_from)
        .transpose()
        .map_err(|e| anyhow::anyhow!("--prompt: {e}"))?;
    let render_quality_value = render_quality
        .map(nolgia_client::types::GenerateImageRequestRenderQuality::try_from)
        .transpose()
        .map_err(|e| anyhow::anyhow!("--render-quality: {e}"))?;
    let body: GenerateImageRequest = GenerateImageRequest::builder()
        .model(model)
        .prompt(prompt)
        .render_quality(render_quality_value)
        .quality(quality)
        .aspect_ratio(aspect_ratio)
        .aura(args.aura)
        .face_reference_asset_id(args.face_reference_asset_id)
        .character_id(args.character_id)
        .reference_asset_ids(reference_asset_ids)
        .mask_asset_id(mask_asset_id)
        .project_id(args.project_id)
        .try_into()
        .context("building image request")?;
    let identity_requested = args.face_reference_asset_id.is_some() || args.character_id.is_some();
    let job = match ctx.client().generate_image().body(body).send().await {
        Ok(response) => response.into_inner(),
        Err(err) => {
            return Err(super::submit_error(err, "submitting image job", "nolgia gen image").await);
        }
    };
    if args.no_wait {
        return livejob::guard(job.id, async {
            print_json(
                ctx.output(),
                &AsyncJob {
                    job_id: job.id.to_string(),
                },
            )
        })
        .await;
    }
    let job_id = job.id;
    livejob::announce(job_id, DEFAULT_WAIT_TIMEOUT_SECONDS);
    livejob::guard(job_id, async move {
        let job = wait_for_asset(job_id, ctx, DEFAULT_WAIT_TIMEOUT_SECONDS).await?;
        let asset = job
            .asset
            .as_ref()
            .context("image job completed without asset")?;
        if let Some(out) = args.out {
            download(&asset.signed_url, &out).await?;
        }
        match ctx.format() {
            OutputFormat::Json => print_json(ctx.output(), &job),
            OutputFormat::Text => {
                println!("{}", asset.signed_url);
                print_identity(asset, identity_requested);
                Ok(())
            }
        }
    })
    .await
}

/// `--prompt` may be left off only on a model the catalog marks as taking
/// none, and such a model works on one source image, which is then what is
/// required. Without a catalog entry the CLI cannot vouch for the model, so
/// the prompt stays required.
fn check_promptless_image(
    model_id: &str,
    entry: Option<&nolgia_client::types::Model>,
    has_input: bool,
) -> Result<()> {
    let Some(flag) = entry.and_then(super::models::promptless_flag) else {
        anyhow::bail!(
            "--prompt is required for {model_id}. Only a model that takes no prompt runs \
             without one, given --input: background removal (`remove background` in `nolgia \
             models list`) or an enhancer (`enhance`)."
        );
    };
    anyhow::ensure!(
        has_input,
        "{model_id} takes no prompt (`{flag}` in `nolgia models get {model_id} --json`): it \
         works on one source image, so pass --input with a local file or an image asset UUID"
    );
    Ok(())
}

/// Report the Aura identity gate's verdict on stderr, so stdout stays the
/// asset URL (or job line) that scripts read. `--json` carries the same
/// fields on `asset`. When an identity was requested but nothing was scored,
/// say why that can happen rather than leave the absence unexplained.
fn print_identity(asset: &nolgia_client::types::Asset, identity_requested: bool) {
    if let Some(line) = identity_line(asset) {
        eprintln!("{line}");
    } else if identity_requested {
        eprintln!(
            "identity: not scored. The face check runs only with consent on record for the \
             reference photo (characters: `nolgia characters update <id> \
             --face-check-consent`) and on models where the scorer is available."
        );
    }
}

fn identity_line(asset: &nolgia_client::types::Asset) -> Option<String> {
    let score = asset.identity_score?;
    let verdict = match asset.identity_gate_passed {
        Some(true) => "passed the 0.60 gate",
        Some(false) => "below the 0.60 gate",
        None => "not gated",
    };
    let rerolls = match asset.identity_rerolls {
        Some(n) if n > 0 => format!(", after {n} automatic re-roll"),
        _ => String::new(),
    };
    let mut line = format!("identity score {score:.3}: {verdict}{rerolls}");
    // A cast's overall score is its weakest member's, so name each member;
    // a single character's own score is the line above.
    let members = asset.character_scores.as_deref().unwrap_or_default();
    for member in members.iter().filter(|_| members.len() > 1) {
        let gate = if member.identity_gate_passed {
            "passed"
        } else {
            "below the gate"
        };
        line.push_str(&format!(
            "\n  character {}: {:.3} ({gate})",
            member.character_id, member.identity_score
        ));
    }
    Some(line)
}

async fn video(mut args: VideoArgs, ctx: &CommandContext) -> Result<()> {
    // Parsed before anything touches the network, so a contradictory
    // duration fails before a catalog read, an upload or a submit.
    let shots = parse_shots(&args.shots)?;
    if let (Some(shots), Some(duration)) = (shots.as_deref(), args.duration_seconds) {
        let shot_total: u64 = shots.iter().map(|s| s.duration_seconds.get()).sum();
        anyhow::ensure!(
            shot_total == duration.get(),
            "--duration-seconds {duration} contradicts the --shot durations \
             (which sum to {shot_total}). The clip length of a multi-shot job is \
             the sum of its shots — omit --duration-seconds, or pass \
             --duration-seconds {shot_total}."
        );
    }
    // Which lane a model belongs to is a catalog fact, not a name list: a
    // background-removal model is submitted to its own route with a source
    // clip and no prompt. An unreadable catalog leaves the ordinary route.
    let entry = super::models::entry(ctx, &args.model).await;
    if entry
        .as_ref()
        .is_some_and(|model| model.remove_background == Some(true))
    {
        return remove_background_video(args, ctx).await;
    }
    let Some(prompt) = args.prompt.take() else {
        return Err(missing_video_prompt(&args.model, entry.as_ref()));
    };
    if args.duration_seconds.is_none() && args.shots.is_empty() && !args.video_refs.is_empty() {
        args.duration_seconds = reference_duration(ctx, entry.as_ref(), &args.video_refs).await;
    }
    if args.cost_only {
        let duration: u64 = match shots.as_deref() {
            None => args.duration_seconds.map(|d| d.get()).unwrap_or(5),
            Some(shots) => shots.iter().map(|s| s.duration_seconds.get()).sum(),
        };
        let quote = super::models::quote_video(
            ctx,
            &args.model.to_string(),
            duration,
            args.quality.as_deref(),
            args.generate_audio,
        )
        .await?;
        println!("{quote}");
        return Ok(());
    }
    // The request schema's ceiling (`video_asset_ids` maxItems). Each
    // model's own cap comes from the catalog in precheck_video_options.
    anyhow::ensure!(
        args.video_refs.len() <= 10,
        "--video-ref: at most 10 reference videos per request"
    );
    anyhow::ensure!(
        args.elements.len() <= 9,
        "--element: at most 9 element images per request"
    );
    anyhow::ensure!(
        args.end_frame.is_none() || args.input.is_some(),
        "--end-frame requires --input (the start frame)"
    );
    // The precheck runs unconditionally now rather than only when a
    // capability flag is present: a model with a MINIMUM reference-audio count
    // (a lip sync route) has to be told it is MISSING one, and an absence is
    // invisible to a "did the caller pass a flag" gate. It still fails open on
    // an unreachable catalog or an unknown model.
    super::models::precheck_video_options(
        ctx,
        &args.model.to_string(),
        &super::models::VideoOptions {
            quality: args.quality.as_deref(),
            bitrate: args.bitrate,
            video_refs: args.video_refs.len(),
            elements: args.elements.len(),
            audio_refs: args.audio_refs.len(),
            end_frame: args.end_frame.is_some(),
        },
    )
    .await?;
    let image_url = match args.input.as_ref() {
        Some(input) => Some(resolve_input_image(input, ctx).await?),
        None => None,
    };
    let end_image_asset_id = match args.end_frame.as_deref() {
        Some(end_frame) => Some(resolve_end_frame(end_frame, ctx).await?),
        None => None,
    };
    // Each --audio-ref resolves to an ASSET ID, never a URL: the API bills a
    // lip sync clip on the track's STORED duration, so it only accepts a
    // track it can measure.
    let mut audio_asset_ids = Vec::with_capacity(args.audio_refs.len());
    for audio_ref in &args.audio_refs {
        audio_asset_ids.push(resolve_audio_reference(audio_ref, ctx).await?);
    }
    let quality = args
        .quality
        .as_deref()
        .map(GenerateVideoRequestQuality::try_from)
        .transpose()
        .map_err(|e| anyhow::anyhow!("--quality: {e}"))?;
    let negative_prompt = args
        .negative_prompt
        .map(GenerateVideoRequestNegativePrompt::try_from)
        .transpose()
        .map_err(|e| anyhow::anyhow!("--negative-prompt: {e}"))?;
    let motion_id = args
        .motion
        .map(GenerateVideoRequestMotionId::try_from)
        .transpose()
        .map_err(|e| anyhow::anyhow!("--motion: {e}"))?;
    let identity_requested = args.character_id.is_some();
    let mut builder = GenerateVideoRequest::builder()
        .model(args.model)
        .prompt(prompt)
        .negative_prompt(negative_prompt)
        .image_url(image_url)
        .end_image_asset_id(end_image_asset_id)
        .aspect_ratio(args.aspect_ratio)
        .seed(args.seed)
        .generate_audio(args.generate_audio)
        .quality(quality)
        .bitrate_mode(args.bitrate)
        .motion_id(motion_id)
        .motion_strength(args.motion_strength)
        .character_id(args.character_id)
        .project_id(args.project_id)
        .shots(shots)
        // Only ever the duration the caller asked for, or the one a single
        // subject reference implies (see reference_duration, which says so on
        // stderr). Left unset the field is omitted entirely and the server
        // derives it — from the shots when there are shots, from its own 5s
        // default when there are not (NOL-342).
        .duration_seconds(args.duration_seconds);
    if !args.video_refs.is_empty() {
        builder = builder.video_asset_ids(Some(args.video_refs));
    }
    if !args.elements.is_empty() {
        builder = builder.element_asset_ids(Some(args.elements));
    }
    if !audio_asset_ids.is_empty() {
        builder = builder.audio_asset_ids(Some(audio_asset_ids));
    }
    let body: GenerateVideoRequest = builder.try_into().context("building video request")?;
    let job = match ctx.client().generate_video().body(body).send().await {
        Ok(response) => response.into_inner(),
        Err(err) => {
            return Err(super::submit_error(err, "submitting video job", "nolgia gen video").await);
        }
    };
    let wait = args.wait && !args.no_wait;
    deliver_video_job(
        job.id,
        wait,
        args.timeout,
        args.out,
        identity_requested,
        ctx,
    )
    .await
}

/// The shared tail of every `gen video` lane: print the job id and return
/// under --no-wait, otherwise wait, save --out and print the outcome.
async fn deliver_video_job(
    job_id: uuid::Uuid,
    wait: bool,
    timeout: u64,
    out: Option<PathBuf>,
    identity_requested: bool,
    ctx: &CommandContext,
) -> Result<()> {
    if !wait {
        return livejob::guard(job_id, async {
            print_json(
                ctx.output(),
                &AsyncJob {
                    job_id: job_id.to_string(),
                },
            )
        })
        .await;
    }
    livejob::announce(job_id, timeout);
    livejob::guard(job_id, async move {
        let job = wait_for_asset(job_id, ctx, timeout).await?;
        if let (Some(asset), Some(out)) = (&job.asset, out.as_ref()) {
            download(&asset.signed_url, out).await?;
        }
        match ctx.format() {
            OutputFormat::Json => print_json(ctx.output(), &job),
            OutputFormat::Text => {
                println!("{} {}", job.id, job.status);
                if let Some(asset) = &job.asset {
                    print_identity(asset, identity_requested);
                }
                Ok(())
            }
        }
    })
    .await
}

/// `gen video` on a background-removal model (`remove_background` in the
/// catalog): `POST /remove-background/video` with the --input clip as the
/// source. The route takes no prompt and none of the generation controls, so
/// those flags are refused by name rather than silently dropped.
async fn remove_background_video(args: VideoArgs, ctx: &CommandContext) -> Result<()> {
    let model = args.model.as_str();
    let refused: Vec<&str> = [
        ("--prompt", args.prompt.is_some()),
        ("--negative-prompt", args.negative_prompt.is_some()),
        ("--aspect-ratio", args.aspect_ratio.is_some()),
        ("--seed", args.seed.is_some()),
        ("--generate-audio", args.generate_audio.is_some()),
        ("--quality", args.quality.is_some()),
        ("--bitrate", args.bitrate.is_some()),
        ("--motion", args.motion.is_some()),
        ("--video-ref", !args.video_refs.is_empty()),
        ("--element", !args.elements.is_empty()),
        ("--audio-ref", !args.audio_refs.is_empty()),
        ("--end-frame", args.end_frame.is_some()),
        ("--shot", !args.shots.is_empty()),
        ("--character-id", args.character_id.is_some()),
        ("--project-id", args.project_id.is_some()),
    ]
    .into_iter()
    .filter_map(|(flag, given)| given.then_some(flag))
    .collect();
    anyhow::ensure!(
        refused.is_empty(),
        "{model} removes the background of one clip and takes no prompt or generation \
         controls (`remove background` in `nolgia models list`): drop {}. It takes --input \
         (the clip), plus --duration-seconds for a URL or a clip whose length is not stored.",
        refused.join(", ")
    );
    let input = args.input.as_deref().with_context(|| {
        format!(
            "{model} needs --input: the clip to cut the subject out of (one of your video \
             asset UUIDs, a local video file, or an https URL)"
        )
    })?;
    let input = super::restore::classify_input(input)?;
    if matches!(input, super::restore::RestoreInput::Url(_)) {
        anyhow::ensure!(
            args.duration_seconds.is_some(),
            "--duration-seconds is required with a URL source: the server cannot measure \
             external media, and the clip length prices the job. Round the clip length up to \
             whole seconds."
        );
    }
    // The route's model enum is closed, so a model the catalog added after
    // this build is named here, before a local file is uploaded for nothing.
    let model_value =
        nolgia_client::types::RemoveBackgroundVideoModel::try_from(model).map_err(|_| {
            anyhow::anyhow!(
                "{model} is a background-removal model this build of the CLI cannot submit — \
                 update the CLI (`nolgia update`, or reinstall)"
            )
        })?;
    if args.cost_only {
        let seconds = remove_background_seconds(&input, args.duration_seconds, ctx).await?;
        let quote = super::models::quote_video(ctx, model, seconds, None, None).await?;
        println!("{quote}");
        return Ok(());
    }
    let (source_asset_id, source_url) = match input {
        super::restore::RestoreInput::Url(url) => (None, Some(url)),
        super::restore::RestoreInput::Asset(id) => (Some(id), None),
        super::restore::RestoreInput::File(path) => {
            let asset = upload_asset_file(&path, ctx, None).await?;
            // The server bills the stored length. A container whose length
            // was not read at upload has none yet, so the submission would be
            // refused; say how to rerun without uploading again.
            anyhow::ensure!(
                asset.duration_seconds.is_some() || args.duration_seconds.is_some(),
                "{} was uploaded as asset {}, but its length is not known yet and the clip \
                 length prices the job: rerun with --input {} --duration-seconds <seconds, \
                 rounded up>",
                path.display(),
                asset.id,
                asset.id
            );
            (Some(asset.id), None)
        }
    };
    let body: nolgia_client::types::RemoveBackgroundVideoRequest =
        nolgia_client::types::RemoveBackgroundVideoRequest::builder()
            .model(model_value)
            .source_asset_id(source_asset_id)
            .source_url(source_url)
            .duration_seconds(args.duration_seconds)
            .try_into()
            .context("building background removal request")?;
    let job = match ctx
        .client()
        .remove_background_video()
        .body(body)
        .send()
        .await
    {
        Ok(response) => response.into_inner(),
        Err(err) => {
            return Err(super::submit_error(
                err,
                "submitting background removal job",
                "nolgia gen video",
            )
            .await);
        }
    };
    let wait = args.wait && !args.no_wait;
    deliver_video_job(job.id, wait, args.timeout, args.out, false, ctx).await
}

/// The clip length a background removal is priced on, for --cost-only: an
/// asset's stored duration rounded up (the server bills that and ignores
/// --duration-seconds), otherwise --duration-seconds.
async fn remove_background_seconds(
    input: &super::restore::RestoreInput,
    declared: Option<std::num::NonZeroU64>,
    ctx: &CommandContext,
) -> Result<u64> {
    if let super::restore::RestoreInput::Asset(id) = input {
        let asset = ctx
            .client()
            .get_asset()
            .id(*id)
            .send()
            .await
            .with_context(|| format!("fetching asset {id}"))?
            .into_inner();
        if let Some(seconds) = asset.duration_seconds.filter(|s| *s > 0.0) {
            return Ok(seconds.ceil() as u64);
        }
    }
    declared.map(|d| d.get()).context(
        "--cost-only needs the clip's length: pass --duration-seconds (rounded up), or give \
         --input as an asset whose length is stored",
    )
}

/// Why `gen video` has no prompt to send, phrased for the lane the catalog
/// puts the model in.
fn missing_video_prompt(
    model_id: &str,
    entry: Option<&nolgia_client::types::Model>,
) -> anyhow::Error {
    if entry.is_some_and(|model| model.restore == Some(true)) {
        return anyhow::anyhow!(
            "{model_id} is a restore model, which takes a source clip and no prompt: run \
             `nolgia restore video --model {model_id} --input <clip>`"
        );
    }
    anyhow::anyhow!(
        "--prompt is required for {model_id}. Only a background-removal model (`remove \
         background` in `nolgia models list`) runs without one, given --input."
    )
}

/// The clip length to send when the caller gave none but attached reference
/// video, on a model whose reference shows the SUBJECT to perform (Seedance
/// reference-to-video; `references.subject_video_reference`). Left unset the
/// server renders its default length, so a 9s driver clip came back 5s. One
/// reference sets the length: its stored duration counted to the nearest
/// whole second, the way the API counts a reference, fitted to the model's
/// range. A model whose output already follows its source clip is left to
/// the server, and anything the CLI cannot measure keeps the server default
/// with a warning instead of a guess.
async fn reference_duration(
    ctx: &CommandContext,
    entry: Option<&nolgia_client::types::Model>,
    video_refs: &[uuid::Uuid],
) -> Option<std::num::NonZeroU64> {
    let model = entry?;
    let references = model.references.as_ref()?;
    if references.subject_video_reference != Some(true)
        || references.output_follows_source_video == Some(true)
    {
        return None;
    }
    let [reference] = video_refs else {
        eprintln!(
            "--duration-seconds not given: with {} --video-ref clips {} renders the server's \
             default length, not the clips' length. Pass --duration-seconds to choose.",
            video_refs.len(),
            model.id
        );
        return None;
    };
    let stored = match ctx.client().get_asset().id(*reference).send().await {
        Ok(response) => response.into_inner().duration_seconds,
        Err(_) => None,
    };
    let Some(seconds) = stored.filter(|s| *s > 0.0) else {
        eprintln!(
            "--duration-seconds not given and the --video-ref clip's length is not stored, so \
             {} renders the server's default length. Pass --duration-seconds to match the \
             reference.",
            model.id
        );
        return None;
    };
    let counted = counted_reference_seconds(seconds);
    let chosen = fit_duration(counted, model.video.as_ref());
    if chosen == counted {
        eprintln!(
            "--duration-seconds {chosen}: the --video-ref clip is {seconds:.2}s, counted to the \
             nearest second as the API counts a reference. Pass --duration-seconds to render \
             another length."
        );
    } else {
        eprintln!(
            "--duration-seconds {chosen}: the --video-ref clip is {seconds:.2}s ({counted}s \
             counted), and {chosen}s is the nearest length {} renders. Pass \
             --duration-seconds to render another length.",
            model.id
        );
    }
    std::num::NonZeroU64::new(chosen.max(1) as u64)
}

/// A reference video's length the way the API counts it: to the nearest
/// whole second, halves away from zero (Go's math.Round in nolgia-api's
/// VideoInputCountedSeconds), and never below one second.
fn counted_reference_seconds(seconds: f64) -> i64 {
    (seconds.round() as i64).max(1)
}

/// Fit a length to what the model renders, the way the server fits its own
/// default: the nearest listed duration (ties go up) on a model with a
/// discrete list, otherwise clamped into [min_duration, max_duration].
fn fit_duration(seconds: i64, video: Option<&nolgia_client::types::VideoCapabilities>) -> i64 {
    let Some(video) = video else {
        return seconds;
    };
    if let Some(nearest) = video
        .durations
        .iter()
        .copied()
        .min_by_key(|listed| ((listed - seconds).abs(), -listed))
    {
        return nearest;
    }
    let mut fitted = seconds;
    if let Some(min) = video.min_duration {
        fitted = fitted.max(min);
    }
    if let Some(max) = video.max_duration {
        fitted = fitted.min(max);
    }
    fitted
}

async fn audio(args: AudioArgs, ctx: &CommandContext) -> Result<()> {
    let voice = args
        .voice
        .map(nolgia_client::types::GenerateAudioRequestVoice::try_from)
        .transpose()
        .map_err(|e| anyhow::anyhow!("--voice: {e}"))?;
    let body: GenerateAudioRequest = GenerateAudioRequest::builder()
        .model(args.model)
        .prompt(args.prompt)
        .voice(voice)
        .format(args.format)
        .project_id(args.project_id)
        .try_into()
        .context("building audio request")?;
    // Audio was the one modality that never went through the RFC 7807 helper,
    // so every server refusal here — including the new duplicate `409` — came
    // out as progenitor's raw `Unexpected Response` debug dump.
    let job = match ctx.client().generate_audio().body(body).send().await {
        Ok(response) => response.into_inner(),
        Err(err) => {
            return Err(super::submit_error(err, "submitting audio job", "nolgia gen audio").await);
        }
    };
    if args.no_wait {
        return livejob::guard(job.id, async {
            print_json(
                ctx.output(),
                &AsyncJob {
                    job_id: job.id.to_string(),
                },
            )
        })
        .await;
    }
    let job_id = job.id;
    livejob::announce(job_id, DEFAULT_WAIT_TIMEOUT_SECONDS);
    livejob::guard(job_id, async move {
        let job = wait_for_asset(job_id, ctx, DEFAULT_WAIT_TIMEOUT_SECONDS).await?;
        let asset = job
            .asset
            .as_ref()
            .context("audio job completed without asset")?;
        if let Some(out) = args.out {
            download(&asset.signed_url, &out).await?;
        }
        match ctx.format() {
            OutputFormat::Json => print_json(ctx.output(), &job),
            OutputFormat::Text => {
                println!("{}", asset.signed_url);
                Ok(())
            }
        }
    })
    .await
}

fn parse_shots(raw: &[String]) -> Result<Option<Vec<VideoShot>>> {
    if raw.is_empty() {
        return Ok(None);
    }
    let mut shots = Vec::with_capacity(raw.len());
    for (i, spec) in raw.iter().enumerate() {
        let (secs, rest) = spec.split_once(':').with_context(|| {
            format!(
                "--shot #{}: expected \"SECONDS:PROMPT\", got {spec:?}",
                i + 1
            )
        })?;
        let duration_seconds: std::num::NonZeroU64 = secs.trim().parse().with_context(|| {
            format!(
                "--shot #{}: {secs:?} is not a positive number of seconds",
                i + 1
            )
        })?;
        let (prompt, audio) = match rest.split_once('|') {
            Some((p, a)) => (p.trim(), Some(a.trim())),
            None => (rest.trim(), None),
        };
        let mut shot = VideoShot::builder()
            .prompt(prompt)
            .duration_seconds(duration_seconds);
        if let Some(a) = audio {
            let audio_direction = nolgia_client::types::VideoShotAudio::try_from(a)
                .map_err(|e| anyhow::anyhow!("--shot #{} audio: {e}", i + 1))?;
            shot = shot.audio(Some(audio_direction));
        }
        shots.push(
            shot.try_into()
                .with_context(|| format!("--shot #{}", i + 1))?,
        );
    }
    Ok(Some(shots))
}

/// --input accepts an asset UUID (reuse with a fresh signed URL) or a
/// local file path (uploaded to /assets).
async fn resolve_input_image(input: &str, ctx: &CommandContext) -> Result<String> {
    if !Path::new(input).exists()
        && let Ok(id) = uuid::Uuid::parse_str(input)
    {
        let asset = ctx
            .client()
            .get_asset()
            .id(id)
            .send()
            .await
            .with_context(|| format!("fetching asset {id}"))?
            .into_inner();
        return Ok(asset.signed_url);
    }
    upload_input_image(&PathBuf::from(input), ctx).await
}

/// A reference image given as either an asset UUID or a local file path,
/// resolved to the ASSET ID rather than a signed URL.
///
/// The id is what `reference_asset_ids` and `mask_asset_id` take, and it is
/// the better half of the contract: the server re-signs the underlying object
/// at execution time, so a job that waits out a backlog never dispatches with
/// an expired credential. A signed URL minted here would have to outlive the
/// queue.
async fn resolve_reference_asset(
    input: &str,
    flag: &str,
    ctx: &CommandContext,
) -> Result<uuid::Uuid> {
    if !Path::new(input).exists() {
        return uuid::Uuid::parse_str(input).with_context(|| {
            format!("{flag}: {input:?} is neither an asset UUID nor an existing file")
        });
    }
    Ok(upload_image_asset(&PathBuf::from(input), ctx, None)
        .await?
        .id)
}

/// --audio-ref accepts an audio asset UUID or a local file path (uploaded
/// first), and always resolves to the ASSET ID.
///
/// The id is not a convenience here, it is the contract: a lip sync clip is
/// billed on the voice track's STORED duration, so the API refuses a raw
/// `audio_urls` entry on those models — it cannot measure what it does not
/// hold. Uploading a local file first is exactly what makes the flag usable
/// from a shell.
async fn resolve_audio_reference(input: &str, ctx: &CommandContext) -> Result<uuid::Uuid> {
    if !Path::new(input).exists() {
        return uuid::Uuid::parse_str(input).with_context(|| {
            format!("--audio-ref: {input:?} is neither an asset UUID nor an existing file")
        });
    }
    Ok(upload_asset_file(&PathBuf::from(input), ctx, None)
        .await?
        .id)
}

/// The mask contract, checked on the bytes before anything is uploaded.
///
/// PNG is not our requirement, it is what carries the alpha channel that says
/// which pixels may change: a JPEG mask has no transparency at all, so it
/// would either fail upstream or repaint everything. Colour type 4 (grey+alpha)
/// and 6 (truecolour+alpha) are the two that carry a real PER-PIXEL channel;
/// a `tRNS` chunk on colour type 0/2/3 is a single transparent colour or a
/// palette table, and a painter that exports one has almost certainly
/// flattened the user's strokes.
///
/// The 33-byte IHDR header answers every question the contract asks, so
/// nothing is decoded: a 4096x4096 RGBA decode is 64 MiB of pixels for two
/// integers and a byte.
struct MaskPng {
    width: u32,
    height: u32,
}

fn read_mask_png(path: &Path) -> Result<MaskPng> {
    let bytes = fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let Some(header) = png_header(&bytes) else {
        anyhow::bail!(
            "--mask: {} is not a PNG — an edit mask must be a PNG, because only PNG carries the \
             alpha channel that marks the region to repaint",
            path.display()
        );
    };
    if !matches!(header.colour_type, 4 | 6) {
        anyhow::bail!(
            "--mask: {} is a PNG with no alpha channel (colour type {}) — the see-through areas \
             are what gets repainted, so export it as an RGBA PNG with transparency enabled \
             rather than a flat black-and-white image",
            path.display(),
            header.colour_type
        );
    }
    Ok(MaskPng {
        width: header.width,
        height: header.height,
    })
}

struct PngHeader {
    width: u32,
    height: u32,
    colour_type: u8,
}

/// Reads a PNG's IHDR chunk: 8-byte signature, 4-byte length, "IHDR", then a
/// 13-byte payload of width, height, bit depth and colour type. Returns None
/// for anything that is not a PNG.
fn png_header(bytes: &[u8]) -> Option<PngHeader> {
    const IHDR_END: usize = 8 + 8 + 13;
    if bytes.len() < IHDR_END || &bytes[..8] != b"\x89PNG\r\n\x1a\n" || &bytes[12..16] != b"IHDR" {
        return None;
    }
    Some(PngHeader {
        width: u32::from_be_bytes(bytes[16..20].try_into().ok()?),
        height: u32::from_be_bytes(bytes[20..24].try_into().ok()?),
        colour_type: bytes[25],
    })
}

/// Dimensions of a reference image, when it is a PNG we can read. None for
/// every other format, which is not an error: the server re-checks the real
/// pixels either way, and refusing a JPEG reference here would be inventing a
/// rule the API does not have.
fn png_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    png_header(bytes).map(|h| (h.width, h.height))
}

/// --end-frame accepts an image asset UUID (sent as `end_image_asset_id`)
/// or a local file path (uploaded to /assets first), mirroring --input.
async fn resolve_end_frame(input: &str, ctx: &CommandContext) -> Result<uuid::Uuid> {
    if !Path::new(input).exists() {
        return uuid::Uuid::parse_str(input).with_context(|| {
            format!("--end-frame: {input:?} is neither an asset UUID nor an existing file")
        });
    }
    Ok(upload_image_asset(&PathBuf::from(input), ctx, None)
        .await?
        .id)
}

async fn upload_input_image(path: &PathBuf, ctx: &CommandContext) -> Result<String> {
    Ok(upload_image_asset(path, ctx, None).await?.signed_url)
}

/// Upload a local image to /assets; shared by `gen --input` and
/// `assets upload`. `project_id` files the new asset into a project at
/// creation (gen input/end-frame uploads pass `None` — only the generated
/// output is filed).
pub(crate) async fn upload_image_asset(
    path: &PathBuf,
    ctx: &CommandContext,
    project_id: Option<uuid::Uuid>,
) -> Result<nolgia_client::types::Asset> {
    use base64::Engine as _;
    let content_type = match path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .as_deref()
    {
        Some("png") => UploadAssetRequestContentType::ImagePng,
        Some("jpg") | Some("jpeg") => UploadAssetRequestContentType::ImageJpeg,
        Some("webp") => UploadAssetRequestContentType::ImageWebp,
        other => anyhow::bail!("unsupported image extension {other:?} (png/jpeg/webp only)"),
    };
    let bytes = fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let body: UploadAssetRequest = UploadAssetRequest::builder()
        .content_type(content_type)
        .data(base64::engine::general_purpose::STANDARD.encode(bytes))
        .project_id(project_id)
        .filename(
            path.file_name()
                .and_then(|n| n.to_str())
                .map(UploadAssetRequestFilename::try_from)
                .transpose()
                .map_err(|e| anyhow::anyhow!("filename: {e}"))?,
        )
        .try_into()
        .context("building asset upload")?;
    Ok(ctx
        .client()
        .upload_asset()
        .body(body)
        .send()
        .await
        .with_context(|| format!("uploading {}", path.display()))?
        .into_inner())
}

/// Map a lowercase file extension to the signed-upload content type used by
/// the `POST /assets/uploads` → PUT → complete flow. Covers the video and
/// audio artifacts the base64 `POST /assets` path can't carry; images are
/// handled separately by [`upload_image_asset`]. Returns `None` for anything
/// unsupported.
fn signed_upload_content_type(ext: &str) -> Option<CreateAssetUploadRequestContentType> {
    use CreateAssetUploadRequestContentType as Ct;
    Some(match ext {
        "glb" => Ct::ModelGltfBinary,
        "mp4" => Ct::VideoMp4,
        "mov" | "qt" => Ct::VideoQuicktime,
        "webm" => Ct::VideoWebm,
        "mp3" => Ct::AudioMpeg,
        "wav" => Ct::AudioWav,
        "ogg" | "oga" => Ct::AudioOgg,
        "weba" => Ct::AudioWebm,
        "m4a" => Ct::AudioMp4,
        _ => return None,
    })
}

/// Upload a local media file to `/assets`, choosing the transport by type.
/// Images take the base64 `POST /assets` path (small, single round-trip);
/// video and audio take the signed-upload flow. This is the path the agent
/// film pipeline needs to deliver a stitched master MP4 — not just its
/// component clips (NOL-109).
pub(crate) async fn upload_asset_file(
    path: &PathBuf,
    ctx: &CommandContext,
    project_id: Option<uuid::Uuid>,
) -> Result<nolgia_client::types::Asset> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase());
    match ext.as_deref() {
        Some("png") | Some("jpg") | Some("jpeg") | Some("webp") => {
            upload_image_asset(path, ctx, project_id).await
        }
        Some(ext) => match signed_upload_content_type(ext) {
            Some(content_type) => upload_via_signed_url(path, ctx, content_type, project_id).await,
            None => anyhow::bail!(
                "unsupported file extension {ext:?} \
                 (images: png/jpeg/webp; video: mp4/mov/webm; audio: mp3/wav/ogg/m4a; 3d: glb)"
            ),
        },
        None => anyhow::bail!(
            "cannot determine content type: {} has no file extension",
            path.display()
        ),
    }
}

/// Upload a large media file (video/audio) via the signed-upload flow:
/// `POST /assets/uploads` mints a short-lived signed PUT URL, the bytes are
/// PUT straight to storage (the API never proxies them, so this handles the
/// hundreds-of-MB masters the base64 path rejects), then
/// `POST /assets/uploads/{id}/complete` verifies the object and flips the
/// asset to `ready`. The PUT must send exactly the declared Content-Type and
/// no Authorization header (the signature covers the content type), so it uses
/// a bare reqwest client rather than the authenticated API client.
async fn upload_via_signed_url(
    path: &PathBuf,
    ctx: &CommandContext,
    content_type: CreateAssetUploadRequestContentType,
    project_id: Option<uuid::Uuid>,
) -> Result<nolgia_client::types::Asset> {
    let bytes = fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let size = std::num::NonZeroU64::new(bytes.len() as u64)
        .with_context(|| format!("{} is empty; nothing to upload", path.display()))?;
    let filename = path
        .file_name()
        .and_then(|n| n.to_str())
        .with_context(|| format!("{} has no usable filename", path.display()))?;
    // Bind the exact MIME string for the PUT before moving the enum into the
    // request builder; the signed URL rejects a mismatched Content-Type.
    let mime = content_type.to_string();

    let body: CreateAssetUploadRequest = CreateAssetUploadRequest::builder()
        .content_type(content_type)
        .size_bytes(size)
        .filename(filename)
        .project_id(project_id)
        .try_into()
        .context("building signed upload request")?;

    let slot = ctx
        .client()
        .create_asset_upload()
        .body(body)
        .send()
        .await
        .with_context(|| format!("starting signed upload for {}", path.display()))?
        .into_inner();

    // PUT the bytes directly to storage. A fresh client keeps the API bearer
    // token off the request (the signed URL needs none) and the Content-Type
    // must match the declaration byte-for-byte.
    let response = reqwest::Client::new()
        .put(&slot.upload_url)
        .header(reqwest::header::CONTENT_TYPE, &mime)
        .body(bytes)
        .send()
        .await
        .with_context(|| format!("uploading {} to storage", path.display()))?;
    if !response.status().is_success() {
        let status = response.status();
        let detail = response.text().await.unwrap_or_default();
        anyhow::bail!("signed upload PUT to storage failed ({status}): {detail}");
    }

    // Use the ClientExt helper rather than the generated builder: the builder
    // sends a bodyless POST with no Content-Length, which the production load
    // balancer rejects with 411 before it reaches the API.
    ctx.client()
        .finish_asset_upload(slot.upload_id)
        .await
        .with_context(|| format!("finalizing upload for {}", path.display()))
}

pub(crate) async fn wait_for_asset(
    job_id: uuid::Uuid,
    ctx: &CommandContext,
    timeout_seconds: u64,
) -> Result<nolgia_client::types::Job> {
    let timeout = std::num::NonZeroU64::new(timeout_seconds)
        .context("--timeout must be greater than zero")?;
    match ctx
        .client()
        .wait_for_job()
        .id(job_id)
        .timeout_seconds(timeout)
        .send()
        .await
    {
        Ok(response) => crate::moderation::ensure_not_moderated(response.into_inner())
            .and_then(crate::canceled::ensure_not_canceled),
        Err(err) => {
            Err(super::wait_error(err, "waiting for generation job", job_id, timeout_seconds).await)
        }
    }
}

/// Save a finished asset to `out`, named for what the server actually sent.
///
/// The extension the caller typed is a guess about a format the model
/// chooses: nano-banana-2.1 delivers JPEG, so `--out still.png` used to leave
/// JPEG bytes under a .png name. When the extension names a different media
/// format than the body is, the file is saved under the right one and stderr
/// says so. An extension that matches, a name with no extension and an
/// extension that is not a media type (`.bin`, `.tmp`) are kept as typed.
/// Returns the path written.
pub(crate) async fn download(url: &str, out: &Path) -> Result<PathBuf> {
    let response = reqwest::get(url)
        .await
        .with_context(|| format!("downloading {url}"))?
        .error_for_status()
        .context("downloading the asset")?;
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let bytes = response.bytes().await?;
    let format = sniffed_format(&bytes).or_else(|| content_type.as_deref().and_then(mime_format));
    let path = path_for_format(out, format);
    if path != out
        && let Some(format) = format
    {
        eprintln!(
            "--out: the file is {}, so it was saved as {} rather than {}",
            format_label(format),
            path.display(),
            out.display()
        );
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    fs::write(&path, bytes).with_context(|| format!("writing {}", path.display()))?;
    Ok(path)
}

/// The media format a body's own signature names, as its usual extension,
/// for the formats the API delivers. The bytes are the authority: a storage
/// object labelled with the wrong Content-Type still saves correctly.
fn sniffed_format(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Some("png");
    }
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Some("jpg");
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return Some("gif");
    }
    if bytes.len() >= 12 && bytes.starts_with(b"RIFF") {
        match &bytes[8..12] {
            b"WEBP" => return Some("webp"),
            b"WAVE" => return Some("wav"),
            _ => {}
        }
    }
    if bytes.len() >= 12 && &bytes[4..8] == b"ftyp" {
        return Some(match &bytes[8..12] {
            b"qt  " => "mov",
            b"M4A " => "m4a",
            b"avif" | b"avis" => "avif",
            b"heic" | b"heix" | b"mif1" | b"msf1" => "heic",
            _ => "mp4",
        });
    }
    if bytes.starts_with(&[0x1A, 0x45, 0xDF, 0xA3]) {
        return Some("webm");
    }
    if bytes.starts_with(b"OggS") {
        return Some("ogg");
    }
    if bytes.starts_with(b"glTF") {
        return Some("glb");
    }
    // ID3-tagged MP3, or a bare MPEG-1/2 Layer III frame header.
    if bytes.starts_with(b"ID3")
        || (bytes.len() >= 2
            && bytes[0] == 0xFF
            && bytes[1] & 0xE0 == 0xE0
            && bytes[1] & 0x06 == 0x02)
    {
        return Some("mp3");
    }
    None
}

/// The format a Content-Type header names, for a body whose signature is not
/// one [`sniffed_format`] knows.
fn mime_format(content_type: &str) -> Option<&'static str> {
    let mime = content_type.split(';').next()?.trim().to_ascii_lowercase();
    Some(match mime.as_str() {
        "image/png" => "png",
        "image/jpeg" | "image/jpg" => "jpg",
        "image/webp" => "webp",
        "image/gif" => "gif",
        "image/avif" => "avif",
        "image/heic" | "image/heif" => "heic",
        "video/mp4" => "mp4",
        "video/quicktime" => "mov",
        "video/webm" => "webm",
        "audio/mpeg" | "audio/mp3" => "mp3",
        "audio/wav" | "audio/x-wav" | "audio/wave" => "wav",
        "audio/ogg" => "ogg",
        "audio/webm" => "weba",
        "audio/mp4" | "audio/x-m4a" => "m4a",
        "model/gltf-binary" => "glb",
        _ => return None,
    })
}

/// Extensions that are already a correct name for a body of `format`. The
/// ISO media family shares one entry because MP4, MOV and M4A are the same
/// container and players open each under the others' names.
fn accepted_extensions(format: &str) -> &'static [&'static str] {
    match format {
        "jpg" => &["jpg", "jpeg", "jpe", "jfif"],
        "mp4" | "mov" | "m4a" => &["mp4", "m4v", "mov", "qt", "m4a"],
        "webm" | "weba" => &["webm", "weba", "mkv"],
        "ogg" => &["ogg", "oga", "ogv", "opus"],
        "heic" => &["heic", "heif"],
        "png" => &["png"],
        "webp" => &["webp"],
        "gif" => &["gif"],
        "avif" => &["avif"],
        "wav" => &["wav"],
        "mp3" => &["mp3"],
        "glb" => &["glb"],
        _ => &[],
    }
}

/// Every extension that claims a media format, so typing one is a claim the
/// body can contradict. Anything else is the caller's own naming and is kept.
const MEDIA_EXTENSIONS: &[&str] = &[
    "png", "jpg", "jpeg", "jpe", "jfif", "webp", "gif", "avif", "heic", "heif", "bmp", "tif",
    "tiff", "mp4", "m4v", "mov", "qt", "m4a", "webm", "weba", "mkv", "ogg", "oga", "ogv", "opus",
    "mp3", "wav", "glb", "gltf",
];

fn path_for_format(requested: &Path, format: Option<&'static str>) -> PathBuf {
    let (Some(format), Some(extension)) = (
        format,
        requested
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase),
    ) else {
        return requested.to_path_buf();
    };
    if accepted_extensions(format).contains(&extension.as_str())
        || !MEDIA_EXTENSIONS.contains(&extension.as_str())
    {
        return requested.to_path_buf();
    }
    requested.with_extension(format)
}

fn format_label(format: &str) -> String {
    match format {
        "jpg" => "JPEG".to_string(),
        "mov" => "QuickTime".to_string(),
        "webm" => "WebM".to_string(),
        "weba" => "WebM audio".to_string(),
        other => other.to_ascii_uppercase(),
    }
}

#[cfg(test)]
mod tests {
    use super::{IMAGE_ASPECT_RATIOS, ImageAspectRatio, parse_image_aspect_ratio};

    /// The hand-written choice list exists only to render a good parse error,
    /// so it must never drift from the enum the API actually publishes. This
    /// reads the vendored OpenAPI spec — the same file codegen builds the
    /// client from — and compares the two, so a spec change that adds or
    /// removes a ratio fails here instead of silently teaching the CLI to
    /// advertise the wrong set.
    #[test]
    fn image_aspect_ratio_choices_match_the_spec() {
        let spec_path = concat!(env!("CARGO_MANIFEST_DIR"), "/../client/openapi.yaml");
        let Ok(spec) = std::fs::read_to_string(spec_path) else {
            // Not available when the crate is consumed outside the workspace.
            return;
        };
        let from_spec = spec_enum_values(&spec, "ImageAspectRatio")
            .expect("ImageAspectRatio enum present in the vendored spec");
        assert_eq!(
            from_spec, IMAGE_ASPECT_RATIOS,
            "IMAGE_ASPECT_RATIOS has drifted from the spec's ImageAspectRatio enum"
        );
    }

    /// Every advertised value must actually parse into the generated enum.
    #[test]
    fn every_advertised_image_aspect_ratio_parses() {
        for value in IMAGE_ASPECT_RATIOS {
            let parsed = parse_image_aspect_ratio(value)
                .unwrap_or_else(|e| panic!("{value:?} is advertised but does not parse: {e}"));
            assert_eq!(&parsed.to_string(), value);
        }
    }

    #[test]
    fn image_size_aliases_are_not_accepted_as_ratios() {
        for alias in ["portrait_16_9", "portrait_1080_1920", "square_hd"] {
            let err = parse_image_aspect_ratio(alias)
                .expect_err("image_size aliases are a different vocabulary");
            assert!(err.contains("9:16"), "error should list the real ratios");
        }
    }

    #[test]
    fn ratios_round_trip_through_display() {
        let ratio = parse_image_aspect_ratio("9:16").expect("9:16 parses");
        assert_eq!(ratio, ImageAspectRatio::X916);
        assert_eq!(ratio.to_string(), "9:16");
    }

    /// Pull `components.schemas.<name>.enum` out of the spec, which avoids a
    /// YAML dependency in the CLI crate for one test.
    ///
    /// Scans only the named schema's own block — everything up to the next
    /// sibling key at the same indentation — so it cannot wander into a later
    /// schema's `enum` if the shape ever changes. Handles both the inline flow
    /// form the spec currently uses (`enum: ['16:9', ...]`) and a block list.
    fn spec_enum_values(spec: &str, schema: &str) -> Option<Vec<String>> {
        let body = spec.split_once(&format!("\n    {schema}:\n"))?.1;
        let block: Vec<&str> = body
            .lines()
            .take_while(|l| l.trim().is_empty() || l.starts_with("      "))
            .collect();
        let enum_line = block
            .iter()
            .position(|l| l.trim_start().starts_with("enum:"))?;
        let rest = block[enum_line]
            .trim_start()
            .trim_start_matches("enum:")
            .trim();

        let values: Vec<String> = if let Some(inline) = rest.strip_prefix('[') {
            inline
                .trim_end_matches(']')
                .split(',')
                .map(|v| v.trim().trim_matches('\'').trim_matches('"').to_string())
                .collect()
        } else {
            block[enum_line + 1..]
                .iter()
                .take_while(|l| l.trim_start().starts_with("- "))
                .map(|l| {
                    l.trim()
                        .trim_start_matches("- ")
                        .trim_matches('\'')
                        .trim_matches('"')
                        .to_string()
                })
                .collect()
        };
        values.iter().all(|v| !v.is_empty()).then_some(values)
    }
}

#[derive(Args, Debug)]
#[command(
    after_help = "Turn photos into a GLB. Hunyuan3D costs 21 credits textured or 13 untextured, plus 9 for PBR and 9 once for extra views. Draft (trellis) costs 2 credits. Agents: estimate with --cost-only first and confirm with the user before submitting batches over ~2000 credits."
)]
pub struct ThreeDArgs {
    /// Image file or asset UUID; repeat in front, back, left, right order (1 to 4)
    #[arg(
        long,
        value_name = "PATH_OR_UUID",
        required_unless_present = "image_url",
        conflicts_with = "image_url"
    )]
    pub input: Vec<String>,
    /// Hosted HTTPS front image instead of --input
    #[arg(long, value_name = "URL", required_unless_present = "input")]
    pub image_url: Option<String>,
    /// Model id; omitted by default so the server selects hunyuan3d-v3
    #[arg(long, value_parser = clap::value_parser!(nolgia_client::types::Generate3DModel), value_name = "hunyuan3d-v3|trellis")]
    pub model: Option<nolgia_client::types::Generate3DModel>,
    /// Use trellis (2 credits, one image, textured only)
    #[arg(long, conflicts_with = "model")]
    pub draft: bool,
    /// Generate an untextured white model (hunyuan3d-v3 only, 13 credits)
    #[arg(long)]
    pub no_texture: bool,
    /// Add PBR materials (hunyuan3d-v3 only, +9 credits)
    #[arg(long, conflicts_with = "no_texture")]
    pub pbr: bool,
    #[arg(long)]
    pub project_id: Option<uuid::Uuid>,
    /// Tag the generated asset; repeat for multiple tags (up to 10)
    #[arg(long)]
    pub tag: Vec<String>,
    /// Download the GLB to this exact path
    #[arg(long)]
    pub out: Option<PathBuf>,
    #[arg(long)]
    pub no_wait: bool,
    #[arg(long, default_value_t = 300)]
    pub timeout: u64,
    /// Print the live catalog credit estimate without uploading or submitting
    #[arg(long)]
    pub cost_only: bool,
}

async fn three_d(args: ThreeDArgs, ctx: &CommandContext) -> Result<()> {
    use nolgia_client::types::{
        Generate3DModel, Generate3DRequest, Generate3DRequestQuality, Generate3DRequestTagsItem,
    };
    anyhow::ensure!(
        args.input.len() <= 4,
        "--input: at most 4 images per request"
    );
    let model = if args.draft {
        Generate3DModel::Trellis
    } else {
        args.model.unwrap_or(Generate3DModel::Hunyuan3dV3)
    };
    if model == Generate3DModel::Trellis {
        anyhow::ensure!(
            args.input.len() <= 1,
            "--input: trellis/--draft requires exactly one image"
        );
        anyhow::ensure!(
            !args.no_texture,
            "--no-texture is not supported by trellis/--draft"
        );
        anyhow::ensure!(!args.pbr, "--pbr is not supported by trellis/--draft");
    }
    let tags = args
        .tag
        .into_iter()
        .map(Generate3DRequestTagsItem::try_from)
        .collect::<Result<Vec<_>, _>>()
        .context("invalid --tag")?;
    anyhow::ensure!(tags.len() <= 10, "--tag: at most 10 tags per request");
    if args.cost_only {
        match super::models::quote_three_d(
            ctx,
            &model.to_string(),
            args.no_texture,
            args.pbr,
            args.input.len() > 1,
        )
        .await
        {
            Ok(quote) => println!("{quote}"),
            Err(err) => println!(
                "3D credit estimate unavailable: {err:#}. No job submitted; try `nolgia models list --modality 3d`."
            ),
        }
        return Ok(());
    }
    let mut image_asset_ids = Vec::with_capacity(args.input.len());
    for input in &args.input {
        image_asset_ids.push(resolve_reference_asset(input, "--input", ctx).await?);
    }
    let body = Generate3DRequest {
        image_asset_ids,
        image_url: args.image_url,
        model: args.model,
        quality: args.draft.then_some(Generate3DRequestQuality::Draft),
        texture: args.no_texture.then_some(false),
        pbr: args.pbr.then_some(true),
        project_id: args.project_id,
        tags,
        ..Default::default()
    };
    let job = match ctx.client().generate3_d().body(body).send().await {
        Ok(response) => response.into_inner(),
        Err(err) => {
            return Err(super::submit_error(err, "submitting 3D job", "nolgia gen 3d").await);
        }
    };
    if args.no_wait {
        return livejob::guard(job.id, async {
            print_json(
                ctx.output(),
                &AsyncJob {
                    job_id: job.id.to_string(),
                },
            )
        })
        .await;
    }
    let job_id = job.id;
    livejob::announce(job_id, args.timeout);
    livejob::guard(job_id, async move {
        let job = wait_for_asset(job_id, ctx, args.timeout).await?;
        let asset = job
            .asset
            .as_ref()
            .context("3D job completed without asset")?;
        if let Some(out) = args.out.as_ref() {
            download(&asset.signed_url, out).await?;
        }
        match ctx.format() {
            OutputFormat::Json => print_json(ctx.output(), &job),
            OutputFormat::Text => {
                println!("{} {}\n{}", job.id, job.status, asset.signed_url);
                Ok(())
            }
        }
    })
    .await
}

#[cfg(test)]
mod three_d_tests {
    #[test]
    fn glb_signed_upload_uses_gltf_binary() {
        assert_eq!(
            super::signed_upload_content_type("glb"),
            Some(super::CreateAssetUploadRequestContentType::ModelGltfBinary)
        );
    }
}

#[cfg(test)]
mod encore_gap_tests {
    use std::path::{Path, PathBuf};

    use nolgia_client::types::{Asset, Model, VideoCapabilities};
    use serde_json::json;

    use super::{
        check_promptless_image, counted_reference_seconds, fit_duration, identity_line,
        mime_format, path_for_format, sniffed_format,
    };

    fn model(mut value: serde_json::Value) -> Model {
        value["recommended"] = json!(false);
        serde_json::from_value(value).expect("model fixture parses")
    }

    fn video(value: serde_json::Value) -> VideoCapabilities {
        serde_json::from_value(value).expect("video capabilities fixture parses")
    }

    #[test]
    fn prompt_may_be_omitted_only_for_promptless_models_with_input() {
        let cutout = model(json!({
            "id": "remove-background", "modality": "image", "remove_background": true
        }));
        let enhancer = model(json!({
            "id": "topaz-image-standard", "modality": "image", "image_enhance": true
        }));
        let generator = model(json!({"id": "flux-pro", "modality": "image"}));

        assert!(check_promptless_image("remove-background", Some(&cutout), true).is_ok());
        assert!(check_promptless_image("topaz-image-standard", Some(&enhancer), true).is_ok());
        let needs_input = check_promptless_image("remove-background", Some(&cutout), false)
            .unwrap_err()
            .to_string();
        assert!(needs_input.contains("--input"), "{needs_input}");
        let needs_prompt = check_promptless_image("flux-pro", Some(&generator), true)
            .unwrap_err()
            .to_string();
        assert!(
            needs_prompt.contains("--prompt is required for flux-pro"),
            "{needs_prompt}"
        );
        // An unreadable catalog cannot vouch for the model.
        assert!(check_promptless_image("remove-background", None, true).is_err());
    }

    #[test]
    fn reference_seconds_round_to_nearest_like_the_api() {
        assert_eq!(counted_reference_seconds(5.25), 5);
        assert_eq!(counted_reference_seconds(5.5), 6);
        assert_eq!(counted_reference_seconds(9.6), 10);
        assert_eq!(counted_reference_seconds(0.3), 1);
    }

    #[test]
    fn fitted_duration_clamps_a_range_and_snaps_a_list() {
        let ranged = video(json!({"min_duration": 4, "max_duration": 30}));
        assert_eq!(fit_duration(9, Some(&ranged)), 9);
        assert_eq!(fit_duration(2, Some(&ranged)), 4);
        assert_eq!(fit_duration(45, Some(&ranged)), 30);
        let listed = video(json!({"durations": [4, 6, 8]}));
        assert_eq!(
            fit_duration(5, Some(&listed)),
            6,
            "ties go up, like the server"
        );
        assert_eq!(fit_duration(7, Some(&listed)), 8);
        assert_eq!(fit_duration(12, Some(&listed)), 8);
        assert_eq!(fit_duration(9, None), 9);
    }

    #[test]
    fn body_signature_names_the_format() {
        assert_eq!(sniffed_format(&[0xFF, 0xD8, 0xFF, 0xE0]), Some("jpg"));
        assert_eq!(sniffed_format(b"\x89PNG\r\n\x1a\n...."), Some("png"));
        assert_eq!(sniffed_format(b"RIFF\0\0\0\0WEBPVP8 "), Some("webp"));
        assert_eq!(sniffed_format(b"\0\0\0\x18ftypisom"), Some("mp4"));
        assert_eq!(sniffed_format(b"\0\0\0\x14ftypqt  "), Some("mov"));
        assert_eq!(
            sniffed_format(&[0x1A, 0x45, 0xDF, 0xA3, 0x01]),
            Some("webm")
        );
        assert_eq!(sniffed_format(b"ID3\x04"), Some("mp3"));
        assert_eq!(sniffed_format(b"glTF\x02"), Some("glb"));
        assert_eq!(sniffed_format(&[1, 2, 3]), None);
        assert_eq!(mime_format("image/jpeg; charset=binary"), Some("jpg"));
        assert_eq!(mime_format("application/octet-stream"), None);
    }

    #[test]
    fn a_wrong_media_extension_is_corrected_and_everything_else_kept() {
        let png = Path::new("out/still.png");
        assert_eq!(
            path_for_format(png, Some("jpg")),
            PathBuf::from("out/still.jpg")
        );
        assert_eq!(path_for_format(png, Some("png")), png);
        assert_eq!(
            path_for_format(Path::new("a.JPEG"), Some("jpg")),
            Path::new("a.JPEG")
        );
        assert_eq!(
            path_for_format(Path::new("cut.mp4"), Some("webm")),
            PathBuf::from("cut.webm")
        );
        // MP4 and MOV are one container; neither name is wrong for the other.
        assert_eq!(
            path_for_format(Path::new("clip.mov"), Some("mp4")),
            Path::new("clip.mov")
        );
        // No extension, or one that is not a media type, is the caller's own.
        assert_eq!(
            path_for_format(Path::new("still"), Some("jpg")),
            Path::new("still")
        );
        assert_eq!(
            path_for_format(Path::new("still.bin"), Some("jpg")),
            Path::new("still.bin")
        );
        assert_eq!(path_for_format(png, None), png);
    }

    fn asset(extra: serde_json::Value) -> Asset {
        let mut value = json!({
            "id": "66666666-6666-4666-8666-666666666666",
            "user_id": "22222222-2222-4222-8222-222222222222",
            "modality": "image", "model": "gpt-image-2.5-sunburst",
            "signed_url": "https://files/a.png",
            "expires_at": "2026-06-13T00:00:00Z", "created_at": "2026-06-13T00:00:00Z"
        });
        value
            .as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        serde_json::from_value(value).expect("asset fixture parses")
    }

    #[test]
    fn identity_line_reports_score_gate_and_rerolls() {
        assert_eq!(identity_line(&asset(json!({}))), None);
        assert_eq!(
            identity_line(&asset(json!({
                "identity_score": 0.812, "identity_gate_passed": true, "identity_rerolls": 0
            })))
            .as_deref(),
            Some("identity score 0.812: passed the 0.60 gate")
        );
        assert_eq!(
            identity_line(&asset(json!({
                "identity_score": 0.55, "identity_gate_passed": false, "identity_rerolls": 1
            })))
            .as_deref(),
            Some("identity score 0.550: below the 0.60 gate, after 1 automatic re-roll")
        );
        let one = "44444444-4444-4444-8444-444444444444";
        let two = "55555555-5555-4555-8555-555555555555";
        assert_eq!(
            identity_line(&asset(json!({
                "identity_score": 0.753, "identity_gate_passed": true,
                "character_scores": [{"character_id": one, "identity_score": 0.753, "identity_gate_passed": true}]
            })))
            .as_deref(),
            Some("identity score 0.753: passed the 0.60 gate"),
            "a single character's score is the headline, not repeated"
        );
        assert_eq!(
            identity_line(&asset(json!({
                "identity_score": 0.58, "identity_gate_passed": false,
                "character_scores": [
                    {"character_id": one, "identity_score": 0.81, "identity_gate_passed": true},
                    {"character_id": two, "identity_score": 0.58, "identity_gate_passed": false}
                ]
            })))
            .as_deref(),
            Some(&*format!(
                "identity score 0.580: below the 0.60 gate\n  character {one}: 0.810 (passed)\n  \
                 character {two}: 0.580 (below the gate)"
            ))
        );
    }
}
