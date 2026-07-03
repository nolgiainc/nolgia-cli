use anyhow::{Context, Result};
use clap::{Args, Subcommand};
use nolgia_client::types::{
    AspectRatio, AudioFormat, AudioModel, GenerateAudioRequest, GenerateImageRequest,
    GenerateVideoRequest, GenerateVideoRequestNegativePrompt, ImageModel, UploadAssetRequest,
    UploadAssetRequestContentType, UploadAssetRequestFilename, VideoModel, VideoShot,
};
use serde::Serialize;
use std::{fs, path::PathBuf};

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
    #[arg(long, default_value_t = false)]
    pub wait: bool,
    #[arg(long, default_value_t = false)]
    pub no_wait: bool,
}

#[derive(Args, Debug)]
pub struct VideoArgs {
    #[arg(long, default_value = "fal-ai/kling-video/v3/text-to-video")]
    pub model: VideoModel,
    #[arg(long)]
    pub prompt: String,
    /// Start image for image-to-video models; uploaded to /assets first.
    #[arg(long)]
    pub input: Option<PathBuf>,
    #[arg(long)]
    pub out: Option<PathBuf>,
    /// e.g. 16:9, 9:16, 1:1, 4:3, 3:4 (model-dependent)
    #[arg(long)]
    pub aspect_ratio: Option<AspectRatio>,
    /// Clip length in seconds (model-dependent; Kling/Seedance 3-15, Veo 4/6/8)
    #[arg(long)]
    pub duration_seconds: Option<std::num::NonZeroU64>,
    #[arg(long)]
    pub seed: Option<u64>,
    #[arg(long)]
    pub negative_prompt: Option<String>,
    /// Generate a synchronized audio track (Seedance/Veo)
    #[arg(long, action = clap::ArgAction::Set)]
    pub generate_audio: Option<bool>,
    /// Multi-shot segment "SECONDS:PROMPT" or "SECONDS:PROMPT|AUDIO DIRECTION".
    /// Repeat up to 8 times; clip duration = sum, --prompt becomes style/context.
    #[arg(long = "shot")]
    pub shots: Vec<String>,
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
    #[arg(long, default_value = "mp3")]
    pub format: AudioFormat,
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
    let body: GenerateImageRequest = GenerateImageRequest::builder()
        .model(args.model)
        .prompt(args.prompt)
        .try_into()
        .context("building image request")?;
    let job = ctx
        .client()
        .generate_image()
        .body(body)
        .send()
        .await
        .context("submitting image job")?
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
    let image_url = match args.input.as_ref() {
        Some(path) => Some(upload_input_image(path, ctx).await?),
        None => None,
    };
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
        .aspect_ratio(args.aspect_ratio)
        .seed(args.seed)
        .generate_audio(args.generate_audio)
        .shots(shots);
    if let Some(duration) = args.duration_seconds {
        builder = builder.duration_seconds(duration);
    }
    let body: GenerateVideoRequest = builder.try_into().context("building video request")?;
    let mut job = ctx
        .client()
        .generate_video()
        .body(body)
        .send()
        .await
        .context("submitting video job")?
        .into_inner();
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
    let body: GenerateAudioRequest = GenerateAudioRequest::builder()
        .model(args.model)
        .prompt(args.prompt)
        .format(args.format)
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

async fn upload_input_image(path: &PathBuf, ctx: &CommandContext) -> Result<String> {
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
        other => anyhow::bail!(
            "unsupported --input extension {:?} (png/jpeg/webp only)",
            other
        ),
    };
    let bytes = fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let body: UploadAssetRequest = UploadAssetRequest::builder()
        .content_type(content_type)
        .data(base64::engine::general_purpose::STANDARD.encode(bytes))
        .filename(
            path.file_name()
                .and_then(|n| n.to_str())
                .map(UploadAssetRequestFilename::try_from)
                .transpose()
                .map_err(|e| anyhow::anyhow!("--input filename: {e}"))?,
        )
        .try_into()
        .context("building asset upload")?;
    let asset = ctx
        .client()
        .upload_asset()
        .body(body)
        .send()
        .await
        .context("uploading --input image")?
        .into_inner();
    Ok(asset.signed_url)
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
