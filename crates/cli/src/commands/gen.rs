use anyhow::{Context, Result};
use clap::{Args, Subcommand};
use nolgia_client::ClientExt;
use nolgia_client::types::{
    AspectRatio, AudioFormat, AudioModel, BitrateMode, CreateAssetUploadRequest,
    CreateAssetUploadRequestContentType, GenerateAudioRequest, GenerateImageRequest,
    GenerateImageRequestQuality, GenerateVideoRequest, GenerateVideoRequestNegativePrompt,
    GenerateVideoRequestQuality, ImageModel, UploadAssetRequest, UploadAssetRequestContentType,
    UploadAssetRequestFilename, VideoModel, VideoShot,
};
use serde::Serialize;
use std::{
    fs,
    path::{Path, PathBuf},
};

use crate::output::{OutputFormat, print_json};

use super::CommandContext;

#[derive(Subcommand, Debug)]
pub enum GenCommand {
    Image(ImageArgs),
    Video(VideoArgs),
    Audio(AudioArgs),
}

#[derive(Args, Debug)]
pub struct ImageArgs {
    #[arg(long, default_value = "flux-pro")]
    pub model: ImageModel,
    #[arg(long)]
    pub prompt: String,
    #[arg(long)]
    pub input: Option<PathBuf>,
    #[arg(long)]
    pub out: Option<PathBuf>,
    /// Quality/resolution tier (model-specific; tiers and per-tier credits
    /// in `nolgia models get <model>`). Omit for the model's default tier.
    #[arg(long)]
    pub quality: Option<String>,
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
    #[arg(long, default_value = "fal-ai/kling-video/v3/text-to-video")]
    pub model: VideoModel,
    #[arg(long)]
    pub prompt: String,
    /// Start image: a local file (uploaded to /assets) or the UUID of an
    /// existing asset (reused, fresh signed URL). Required for
    /// image-to-video models; optional on models with image input
    /// support (Veo, Omni Flash) per `nolgia models list`.
    #[arg(long)]
    pub input: Option<String>,
    #[arg(long)]
    pub out: Option<PathBuf>,
    /// e.g. 16:9, 9:16, 1:1, 4:3, 3:4 (model-dependent)
    #[arg(long)]
    pub aspect_ratio: Option<AspectRatio>,
    /// Clip length in seconds (model-dependent; Kling/Seedance 3-15, Veo 4/6/8, Omni Flash 3-10)
    #[arg(long)]
    pub duration_seconds: Option<std::num::NonZeroU64>,
    #[arg(long)]
    pub seed: Option<u64>,
    #[arg(long)]
    pub negative_prompt: Option<String>,
    /// Generate a synchronized audio track (Seedance/Veo)
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
    /// Reference video for reference-to-video models: the UUID of one of
    /// your video assets (repeat up to 3). Address them in the prompt as
    /// @Video1..@Video3. Inputs: MP4/MOV, 480p-720p, 2-15s and 50MB
    /// combined across all reference videos.
    #[arg(long = "video-ref", value_name = "ASSET_ID")]
    pub video_refs: Vec<uuid::Uuid>,
    /// Element/reference image for reference-to-video models: the UUID of
    /// one of your image assets (repeat up to 9). Address them in the
    /// prompt as @Image1..@Image9.
    #[arg(long = "element", value_name = "ASSET_ID")]
    pub elements: Vec<uuid::Uuid>,
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
    #[arg(long, default_value = "fal-ai/stable-audio-25/text-to-audio")]
    pub model: AudioModel,
    #[arg(long)]
    pub prompt: String,
    #[arg(long)]
    pub input: Option<PathBuf>,
    #[arg(long)]
    pub out: Option<PathBuf>,
    /// Voice id for TTS models (see `nolgia models get <model>`)
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

#[derive(Serialize)]
struct AsyncJob {
    job_id: String,
}

const DEFAULT_WAIT_TIMEOUT_SECONDS: u64 = 300;

pub async fn run(command: GenCommand, ctx: &CommandContext) -> Result<()> {
    match command {
        GenCommand::Image(args) => image(args, ctx).await,
        GenCommand::Video(args) => video(args, ctx).await,
        GenCommand::Audio(args) => audio(args, ctx).await,
    }
}

async fn image(args: ImageArgs, ctx: &CommandContext) -> Result<()> {
    if let Some(tier) = args.quality.as_deref() {
        super::models::precheck_image_quality(ctx, &args.model.to_string(), tier).await?;
    }
    let quality = args
        .quality
        .as_deref()
        .map(GenerateImageRequestQuality::try_from)
        .transpose()
        .map_err(|e| anyhow::anyhow!("--quality: {e}"))?;
    let body: GenerateImageRequest = GenerateImageRequest::builder()
        .model(args.model)
        .prompt(args.prompt)
        .quality(quality)
        .project_id(args.project_id)
        .try_into()
        .context("building image request")?;
    let job = match ctx.client().generate_image().body(body).send().await {
        Ok(response) => response.into_inner(),
        Err(err) => return Err(super::api_error(err, "submitting image job").await),
    };
    if args.no_wait {
        return print_json(&AsyncJob {
            job_id: job.id.to_string(),
        });
    }
    let job = wait_for_asset(job.id, ctx, DEFAULT_WAIT_TIMEOUT_SECONDS).await?;
    let asset = job
        .asset
        .as_ref()
        .context("image job completed without asset")?;
    if let Some(out) = args.out {
        download(&asset.signed_url, &out).await?;
    }
    match ctx.format() {
        OutputFormat::Json => print_json(&job),
        OutputFormat::Text => {
            println!("{}", asset.signed_url);
            Ok(())
        }
    }
}

async fn video(args: VideoArgs, ctx: &CommandContext) -> Result<()> {
    if args.cost_only {
        let duration: u64 = if args.shots.is_empty() {
            args.duration_seconds.map(|d| d.get()).unwrap_or(5)
        } else {
            parse_shots(&args.shots)?
                .unwrap_or_default()
                .iter()
                .map(|s| s.duration_seconds.get())
                .sum()
        };
        let quote = super::models::quote_video(
            ctx,
            &args.model.to_string(),
            duration,
            args.quality.as_deref(),
        )
        .await?;
        println!("{quote}");
        return Ok(());
    }
    anyhow::ensure!(
        args.video_refs.len() <= 3,
        "--video-ref: at most 3 reference videos per request"
    );
    anyhow::ensure!(
        args.elements.len() <= 9,
        "--element: at most 9 element images per request"
    );
    anyhow::ensure!(
        args.end_frame.is_none() || args.input.is_some(),
        "--end-frame requires --input (the start frame)"
    );
    let uses_capability_flags = args.quality.is_some()
        || args.bitrate.is_some()
        || args.end_frame.is_some()
        || !args.video_refs.is_empty()
        || !args.elements.is_empty();
    if uses_capability_flags {
        super::models::precheck_video_options(
            ctx,
            &args.model.to_string(),
            &super::models::VideoOptions {
                quality: args.quality.as_deref(),
                bitrate: args.bitrate,
                video_refs: args.video_refs.len(),
                elements: args.elements.len(),
                end_frame: args.end_frame.is_some(),
            },
        )
        .await?;
    }
    let image_url = match args.input.as_ref() {
        Some(input) => Some(resolve_input_image(input, ctx).await?),
        None => None,
    };
    let end_image_asset_id = match args.end_frame.as_deref() {
        Some(end_frame) => Some(resolve_end_frame(end_frame, ctx).await?),
        None => None,
    };
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
    let shots = parse_shots(&args.shots)?;
    let mut builder = GenerateVideoRequest::builder()
        .model(args.model)
        .prompt(args.prompt)
        .negative_prompt(negative_prompt)
        .image_url(image_url)
        .end_image_asset_id(end_image_asset_id)
        .aspect_ratio(args.aspect_ratio)
        .seed(args.seed)
        .generate_audio(args.generate_audio)
        .quality(quality)
        .bitrate_mode(args.bitrate)
        .project_id(args.project_id)
        .shots(shots);
    if let Some(duration) = args.duration_seconds {
        builder = builder.duration_seconds(duration);
    }
    if !args.video_refs.is_empty() {
        builder = builder.video_asset_ids(Some(args.video_refs));
    }
    if !args.elements.is_empty() {
        builder = builder.element_asset_ids(Some(args.elements));
    }
    let body: GenerateVideoRequest = builder.try_into().context("building video request")?;
    let mut job = match ctx.client().generate_video().body(body).send().await {
        Ok(response) => response.into_inner(),
        Err(err) => return Err(super::api_error(err, "submitting video job").await),
    };
    if args.no_wait || !args.wait {
        return print_json(&AsyncJob {
            job_id: job.id.to_string(),
        });
    }
    job = wait_for_asset(job.id, ctx, args.timeout).await?;
    if let (Some(asset), Some(out)) = (&job.asset, args.out.as_ref()) {
        download(&asset.signed_url, out).await?;
    }
    match ctx.format() {
        OutputFormat::Json => print_json(&job),
        OutputFormat::Text => {
            println!("{} {}", job.id, job.status);
            Ok(())
        }
    }
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
    let job = ctx
        .client()
        .generate_audio()
        .body(body)
        .send()
        .await
        .context("submitting audio job")?
        .into_inner();
    if args.no_wait {
        return print_json(&AsyncJob {
            job_id: job.id.to_string(),
        });
    }
    let job = wait_for_asset(job.id, ctx, DEFAULT_WAIT_TIMEOUT_SECONDS).await?;
    let asset = job
        .asset
        .as_ref()
        .context("audio job completed without asset")?;
    if let Some(out) = args.out {
        download(&asset.signed_url, &out).await?;
    }
    match ctx.format() {
        OutputFormat::Json => print_json(&job),
        OutputFormat::Text => {
            println!("{}", asset.signed_url);
            Ok(())
        }
    }
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
                 (images: png/jpeg/webp; video: mp4/mov/webm; audio: mp3/wav/ogg/m4a)"
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

async fn wait_for_asset(
    job_id: uuid::Uuid,
    ctx: &CommandContext,
    timeout_seconds: u64,
) -> Result<nolgia_client::types::Job> {
    let timeout = std::num::NonZeroU64::new(timeout_seconds)
        .context("--timeout must be greater than zero")?;
    ctx.client()
        .wait_for_job()
        .id(job_id)
        .timeout_seconds(timeout)
        .send()
        .await
        .context("waiting for generation job")
        .map(|response| response.into_inner())
}

pub(crate) async fn download(url: &str, out: &PathBuf) -> Result<()> {
    let bytes = reqwest::get(url)
        .await
        .with_context(|| format!("downloading {url}"))?
        .bytes()
        .await?;
    if let Some(parent) = out.parent() {
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    fs::write(out, bytes).with_context(|| format!("writing {}", out.display()))
}
