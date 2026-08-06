use anyhow::{Context, Result};
use clap::{Args, Subcommand};
use nolgia_client::types::{RestoreVideoRequest, RestoreVideoRequestQuality};
use std::path::{Path, PathBuf};

use crate::livejob;
use crate::output::{OutputFormat, print_json};

use super::CommandContext;
use super::r#gen::{AsyncJob, download, upload_asset_file, wait_for_asset};

#[derive(Subcommand, Debug)]
pub enum RestoreCommand {
    Video(RestoreVideoArgs),
}

/// The AI footage-restorer lane: re-renders existing footage at a target
/// resolution tier with de-noise, de-haze and detail recovery. Restore models
/// take no prompt; the source clip and the chosen engine are the whole input.
#[derive(Args, Debug)]
#[command(after_help = "Restore jobs cost credits that scale with the source \
clip length and the target resolution tier, and the per-tier rates differ per \
engine (see `nolgia models get <model>`). Agents: check the tier rates first \
and confirm with the user before restoring long footage at 1440p or above.")]
pub struct RestoreVideoArgs {
    /// Restore engine id (restore-lane models are marked `restore` in
    /// `nolgia models list --modality video`).
    ///
    /// `seedvr2-restore` is the general AI footage restorer. The `topaz-*`
    /// ids are the Topaz master upscalers, one engine each, so pick the one
    /// that matches the footage: `topaz-proteus` (general live action),
    /// `topaz-rhea` (texture-heavy), `topaz-iris` (faces), `topaz-nyx`
    /// (denoise plus detail), `topaz-theia` (fine detail), `topaz-artemis`
    /// (high-quality restore), `topaz-gaia` (CG and animation),
    /// `topaz-dione` (interlaced), and the generative
    /// `topaz-starlight-fast`, `topaz-starlight`, `topaz-wonder`,
    /// `topaz-hyperion`.
    ///
    /// Any id the API accepts is forwarded verbatim and validated
    /// server-side; the API is the authority on what exists.
    #[arg(long, default_value = "seedvr2-restore")]
    pub model: String,
    /// The footage to restore: the UUID of one of your video assets, a local
    /// video file (uploaded to /assets first), or a raw https URL.
    #[arg(long)]
    pub input: String,
    #[arg(long)]
    pub out: Option<PathBuf>,
    /// Target output resolution tier. Omit for the model's default (1080p on
    /// every restore-lane model).
    ///
    /// 720p, 1080p, 1440p and 2160p exist on every restore model; 4320p (8K)
    /// is additionally published by the eight classic Topaz engines
    /// (`topaz-proteus`, `topaz-rhea`, `topaz-iris`, `topaz-nyx`,
    /// `topaz-theia`, `topaz-artemis`, `topaz-gaia`, `topaz-dione`) and by
    /// `topaz-starlight-fast`; `topaz-starlight`, `topaz-wonder` and
    /// `topaz-hyperion` top out at 2160p.
    ///
    /// The tiers and their credit rates are per-model: read the real list
    /// from `nolgia models get <model>`. A tier the selected model does not
    /// publish is refused by the API before the job is created.
    #[arg(long)]
    pub quality: Option<String>,
    /// Detail-injection strength 0..1 on seedvr2-restore (the provider's
    /// noise_scale; default 0.1). Higher values recover more texture but
    /// hallucinate more; keep low for archival fidelity. The Topaz engines
    /// expose no equivalent control — the engine choice and the resolution
    /// tier are their whole input.
    #[arg(long)]
    pub noise_scale: Option<f64>,
    /// Source clip length in seconds (round up). Required for raw URL
    /// sources and assets without stored duration metadata; it prices the
    /// job before it runs. Ignored when the asset's stored duration is known.
    #[arg(long)]
    pub duration_seconds: Option<std::num::NonZeroU64>,
    /// Source clip frame rate. The provider bills per OUTPUT frame, so a
    /// 60 fps clip costs twice a 30 fps clip of the same length. Values at or
    /// below 30 are billed at the 30 fps basis, so declaring a slower rate
    /// never buys a discount; above 60 is refused.
    ///
    /// Required on the `topaz-*` engines, whose rates price the 30 fps basis
    /// exactly and which therefore cannot absorb an undeclared faster source.
    /// Optional on `seedvr2-restore`. The server cannot measure a clip's true
    /// rate, so it refuses to guess rather than under-reserve the job.
    #[arg(long)]
    pub source_fps: Option<std::num::NonZeroU64>,
    #[arg(long)]
    pub seed: Option<u64>,
    /// File the restored asset into this project (`nolgia projects list`
    /// for ids). The project must exist and belong to you.
    #[arg(long, value_name = "PROJECT_UUID")]
    pub project_id: Option<uuid::Uuid>,
    #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
    pub wait: bool,
    #[arg(long, default_value_t = false)]
    pub no_wait: bool,
    /// Seconds to wait for the restore to finish. Restores re-render every
    /// frame, so long or high-tier sources take longer than generations.
    #[arg(long, default_value_t = 600)]
    pub timeout: u64,
}

pub async fn run(command: RestoreCommand, ctx: &CommandContext) -> Result<()> {
    match command {
        RestoreCommand::Video(args) => video(args, ctx).await,
    }
}

/// What `--input` names, decided without touching the network: classifying
/// first is what lets every client-side check run *before* a local file is
/// uploaded, so a rejected option cannot leave an unused asset behind.
enum RestoreInput {
    Url(String),
    Asset(uuid::Uuid),
    File(PathBuf),
}

/// How the request carries the source: a raw URL is forwarded as
/// `source_url`, anything else becomes a `source_asset_id` (an existing
/// asset's UUID, or the asset created by uploading a local file).
enum RestoreSource {
    Url(String),
    Asset(uuid::Uuid),
}

fn classify_input(input: &str) -> Result<RestoreInput> {
    if input.starts_with("https://") || input.starts_with("http://") {
        return Ok(RestoreInput::Url(input.to_string()));
    }
    if Path::new(input).exists() {
        return Ok(RestoreInput::File(PathBuf::from(input)));
    }
    let id = uuid::Uuid::parse_str(input).with_context(|| {
        format!("--input: {input:?} is not an https URL, an asset UUID, or an existing file")
    })?;
    Ok(RestoreInput::Asset(id))
}

fn build_body(
    args: &RestoreVideoArgs,
    quality: Option<&RestoreVideoRequestQuality>,
    source: &RestoreSource,
) -> Result<RestoreVideoRequest> {
    let (source_asset_id, source_url) = match source {
        RestoreSource::Asset(id) => (Some(*id), None),
        RestoreSource::Url(url) => (None, Some(url.clone())),
    };
    RestoreVideoRequest::builder()
        .model(args.model.clone())
        .source_asset_id(source_asset_id)
        .source_url(source_url)
        .quality(quality.cloned())
        .noise_scale(args.noise_scale)
        .duration_seconds(args.duration_seconds)
        .source_fps(args.source_fps)
        .seed(args.seed)
        .project_id(args.project_id)
        .try_into()
        .context("building restore request")
}

async fn video(args: RestoreVideoArgs, ctx: &CommandContext) -> Result<()> {
    let input = classify_input(&args.input)?;
    // Both of these sources have to be priced from a declared clip length,
    // and both refusals have to happen before any bytes move: a URL cannot be
    // measured server-side at all, and a freshly uploaded asset's duration is
    // probed asynchronously (`Asset.duration_seconds` is null until the probe
    // lands), so the submission that follows the upload would be rejected for
    // a missing duration after the whole file was on the wire. An existing
    // asset needs nothing: when its stored duration is known the server bills
    // from that and ignores whatever we send.
    match &input {
        RestoreInput::Url(_) => anyhow::ensure!(
            args.duration_seconds.is_some(),
            "--duration-seconds is required with a URL source: the server cannot \
             measure external media, and the clip length prices the job. Round \
             the source duration up to whole seconds."
        ),
        RestoreInput::File(_) => anyhow::ensure!(
            args.duration_seconds.is_some(),
            "--duration-seconds is required when --input is a local file: the \
             upload's duration is probed asynchronously and is not known yet \
             when the restore is submitted, and the clip length prices the job. \
             Round the source duration up to whole seconds."
        ),
        RestoreInput::Asset(_) => {}
    }
    let quality = args
        .quality
        .as_deref()
        .map(RestoreVideoRequestQuality::try_from)
        .transpose()
        .map_err(|e| anyhow::anyhow!("--quality: {e}"))?;
    // Run the generated request's own bounds checks (noise_scale,
    // duration_seconds, ...) against a placeholder source before uploading,
    // for the same reason: a rejected option must not cost an upload and an
    // orphaned asset. The built value is discarded; the real one is built
    // below once the source id exists.
    if matches!(input, RestoreInput::File(_)) {
        build_body(
            &args,
            quality.as_ref(),
            &RestoreSource::Asset(uuid::Uuid::nil()),
        )?;
    }
    let source = match input {
        RestoreInput::Url(url) => RestoreSource::Url(url),
        RestoreInput::Asset(id) => RestoreSource::Asset(id),
        RestoreInput::File(path) => {
            RestoreSource::Asset(upload_asset_file(&path, ctx, None).await?.id)
        }
    };
    let body = build_body(&args, quality.as_ref(), &source)?;
    let job = match ctx.client().restore_video().body(body).send().await {
        Ok(response) => response.into_inner(),
        Err(err) => {
            return Err(
                super::submit_error(err, "submitting restore job", "nolgia restore video").await,
            );
        }
    };
    if args.no_wait || !args.wait {
        return print_json(&AsyncJob {
            job_id: job.id.to_string(),
        });
    }
    let job_id = job.id;
    livejob::announce(job_id, args.timeout);
    livejob::guard(job_id, async move {
        let job = wait_for_asset(job_id, ctx, args.timeout).await?;
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
    })
    .await
}
