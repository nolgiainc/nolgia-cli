//! Live model catalog: the server is the source of truth for models,
//! capabilities, and pricing — new models appear here with no CLI release.

use anyhow::{Context, Result};
use clap::{Args, Subcommand, ValueEnum};
use nolgia_client::types::{BitrateMode, ImageAspectRatio, Model, QualityCapabilities};

use super::CommandContext;
use crate::output::{OutputFormat, print_json};

#[derive(Subcommand, Debug)]
pub enum ModelsCommand {
    /// List available models with capabilities and credit pricing
    List(ListArgs),
    /// Show one model's capabilities, pricing, and (for audio) voices
    Get(GetArgs),
}

#[derive(Args, Debug)]
pub struct ListArgs {
    #[arg(long, value_enum)]
    pub modality: Option<ModalityFilter>,
}

#[derive(Args, Debug)]
pub struct GetArgs {
    pub id: String,
}

#[derive(ValueEnum, Clone, Copy, Debug)]
pub enum ModalityFilter {
    Image,
    Video,
    Audio,
}

impl ModalityFilter {
    /// Compared as a string: the catalog's `modality` is deserialized
    /// permissively (see `relax_response_only_enums` in the client's
    /// build.rs), so a modality this CLI predates filters out cleanly instead
    /// of failing the whole catalog.
    fn matches(self, modality: &str) -> bool {
        matches!(
            (self, modality),
            (Self::Image, "image") | (Self::Video, "video") | (Self::Audio, "audio")
        )
    }
}

pub async fn run(command: ModelsCommand, ctx: &CommandContext) -> Result<()> {
    match command {
        ModelsCommand::List(args) => list(args, ctx).await,
        ModelsCommand::Get(args) => get(args, ctx).await,
    }
}

async fn fetch(ctx: &CommandContext) -> Result<Vec<Model>> {
    Ok(ctx
        .client()
        .list_models()
        .send()
        .await
        .context("fetching model catalog")?
        .into_inner()
        .models)
}

fn cost_line(model: &Model) -> String {
    match &model.cost {
        Some(cost) => {
            let unit = cost.unit.to_lowercase();
            match cost.baseline_seconds {
                Some(base) => format!("{} credits per {base}s clip", cost.credits),
                None => format!("{} credits ({unit})", cost.credits),
            }
        }
        None => "pricing pending".to_string(),
    }
}

fn capability_line(model: &Model) -> String {
    if let Some(video) = &model.video {
        let mut parts: Vec<String> = Vec::new();
        if !video.durations.is_empty() {
            parts.push(
                video
                    .durations
                    .iter()
                    .map(|d| d.to_string())
                    .collect::<Vec<_>>()
                    .join("/")
                    + "s",
            );
        } else if let (Some(min), Some(max)) = (video.min_duration, video.max_duration) {
            parts.push(format!("{min}-{max}s"));
        }
        let ratios = video
            .aspect_ratios
            .iter()
            .map(|r| format!("{r:?}").replace('N', "").replace('_', ":"))
            .collect::<Vec<_>>()
            .join(" ");
        if !ratios.is_empty() {
            parts.push(ratios);
        }
        if video.image_input == Some(true) {
            parts.push("image-input".to_string());
        }
        if let Some(quality) = &model.quality {
            parts.push(quality_summary(quality));
        }
        if let Some(refs) = &model.references {
            if refs.end_frame {
                parts.push("end-frame".to_string());
            }
            if refs.video_refs_max > 0 {
                parts.push(format!("video-refs:{}", refs.video_refs_max));
            }
            if refs.element_refs_max > 0 {
                parts.push(format!("elements:{}", refs.element_refs_max));
            }
            if !refs.bitrate_modes.is_empty() {
                parts.push("bitrate".to_string());
            }
        }
        return parts.join("  ");
    }
    if let Some(image) = &model.image {
        let mut parts: Vec<String> = Vec::new();
        if !image.aspect_ratios.is_empty() {
            parts.push(
                image
                    .aspect_ratios
                    .iter()
                    .map(|r| r.to_string())
                    .collect::<Vec<_>>()
                    .join(" "),
            );
        }
        if let Some(quality) = &model.quality {
            parts.push(quality_summary(quality));
        }
        if !parts.is_empty() {
            return parts.join("  ");
        }
    }
    if let Some(audio) = &model.audio
        && !audio.voices.is_empty()
    {
        return format!("{} voices", audio.voices.len());
    }
    if let Some(quality) = &model.quality {
        return quality_summary(quality);
    }
    String::new()
}

/// Compact tier summary for one-line listings, e.g. `720p/1080p/4k*`
/// (`*` marks premium tiers).
fn quality_summary(quality: &QualityCapabilities) -> String {
    quality
        .options
        .iter()
        .map(|o| {
            if o.premium {
                format!("{}*", o.id)
            } else {
                o.id.clone()
            }
        })
        .collect::<Vec<_>>()
        .join("/")
}

/// Per-tier pricing detail, e.g. `720p — 240 credits per 5s clip (default)`.
fn quality_lines(model: &Model, quality: &QualityCapabilities) -> Vec<String> {
    let unit = match model.cost.as_ref().and_then(|c| c.baseline_seconds) {
        Some(base) => format!("credits per {base}s clip"),
        None => "credits".to_string(),
    };
    quality
        .options
        .iter()
        .map(|option| {
            let mut line = format!("{} — {} {unit}", option.id, option.credits);
            if quality.default.as_deref() == Some(option.id.as_str()) {
                line.push_str(" (default)");
            }
            if option.premium {
                line.push_str(" (premium)");
            }
            line
        })
        .collect()
}

fn bitrate_list(modes: &[String]) -> String {
    modes.join("|")
}

/// The tiers a model offers, phrased for error messages: credits and
/// premium/default markers included so the user can pick without another
/// lookup.
fn quality_choices(model: &Model, quality: &QualityCapabilities) -> String {
    quality_lines(model, quality).join(", ")
}

/// Estimate the credit charge for a video generation from the live catalog.
/// Mirrors the server: ceil(credits * duration / baseline_seconds), where
/// `credits` is the selected quality tier's rate when `--quality` is given.
pub async fn quote_video(
    ctx: &CommandContext,
    model_id: &str,
    duration_seconds: u64,
    quality: Option<&str>,
) -> Result<String> {
    let models = fetch(ctx).await?;
    let model = models.iter().find(|m| m.id == model_id).with_context(|| {
        format!("model {model_id:?} not in the catalog — see `nolgia models list`")
    })?;
    let Some(cost) = &model.cost else {
        return Ok(format!(
            "{model_id}: pricing pending — the server will quote at submit time"
        ));
    };
    let (base_credits, tier_note) = match quality {
        Some(tier) => {
            let option = check_quality(model, tier)?;
            (option.credits, format!(", {tier}"))
        }
        None => (cost.credits, String::new()),
    };
    let credits = match cost.baseline_seconds {
        Some(base) => (base_credits.get() * duration_seconds).div_ceil(base.get()),
        None => base_credits.get(),
    };
    Ok(format!(
        "{credits} credits ({model_id}, {duration_seconds}s{tier_note})"
    ))
}

/// Reference/quality parameters of a pending `gen video` request, for
/// pre-validation against the catalog.
pub struct VideoOptions<'a> {
    pub quality: Option<&'a str>,
    pub bitrate: Option<BitrateMode>,
    pub video_refs: usize,
    pub elements: usize,
    pub end_frame: bool,
}

/// Best-effort pre-validation of quality/reference options against the live
/// catalog, producing friendlier errors than the server's 400 (e.g. listing
/// the model's tiers with credits). Never blocks on catalog problems — an
/// unreachable catalog or unknown model falls through to server validation.
pub async fn precheck_video_options(
    ctx: &CommandContext,
    model_id: &str,
    options: &VideoOptions<'_>,
) -> Result<()> {
    let Ok(models) = fetch(ctx).await else {
        return Ok(());
    };
    let Some(model) = models.iter().find(|m| m.id == model_id) else {
        return Ok(());
    };
    if let Some(tier) = options.quality {
        check_quality(model, tier)?;
    }
    let Some(refs) = &model.references else {
        return Ok(());
    };
    if options.video_refs as u64 > refs.video_refs_max {
        anyhow::bail!(match refs.video_refs_max {
            0 => format!(
                "--video-ref: {model_id} takes no reference videos — use a \
                 reference-to-video model (see `nolgia models list`)"
            ),
            max => format!("--video-ref: {model_id} accepts at most {max} reference videos"),
        });
    }
    if options.elements as u64 > refs.element_refs_max {
        anyhow::bail!(match refs.element_refs_max {
            0 => format!("--element: {model_id} takes no element/reference images"),
            max => format!("--element: {model_id} accepts at most {max} element images"),
        });
    }
    if options.end_frame && !refs.end_frame {
        anyhow::bail!(
            "--end-frame: {model_id} does not support end-frame pinning \
             (see `references.end_frame` in `nolgia models get {model_id}`)"
        );
    }
    if let Some(mode) = options.bitrate
        && !refs.bitrate_modes.iter().any(|m| *m == mode.to_string())
    {
        anyhow::bail!(match refs.bitrate_modes.is_empty() {
            true => format!("--bitrate: {model_id} has no bitrate selection"),
            false => format!(
                "--bitrate: {model_id} supports {}",
                bitrate_list(&refs.bitrate_modes)
            ),
        });
    }
    Ok(())
}

/// Validate a quality tier against the model's published options; on a miss,
/// list the available tiers with credits (premium/default marked).
pub fn check_quality<'m>(
    model: &'m Model,
    tier: &str,
) -> Result<&'m nolgia_client::types::QualityOption> {
    let Some(quality) = &model.quality else {
        anyhow::bail!(
            "--quality: {} has no quality tiers — omit --quality for its single fixed quality",
            model.id
        );
    };
    quality
        .options
        .iter()
        .find(|o| o.id == tier)
        .with_context(|| {
            format!(
                "--quality {tier:?}: not a tier of {}; available: {}",
                model.id,
                quality_choices(model, quality)
            )
        })
}

/// Best-effort `--quality` pre-validation for image models (the mechanism is
/// live, but no catalog image model declares tiers yet — expect the
/// no-tiers error until one does). Skips silently on catalog problems.
pub async fn precheck_image_quality(
    ctx: &CommandContext,
    model_id: &str,
    tier: &str,
) -> Result<()> {
    let Ok(models) = fetch(ctx).await else {
        return Ok(());
    };
    let Some(model) = models.iter().find(|m| m.id == model_id) else {
        return Ok(());
    };
    check_quality(model, tier).map(|_| ())
}

/// Pre-validate `--aspect-ratio` against the ratios the selected image model
/// actually publishes.
///
/// The spec's `ImageAspectRatio` enum is the union across every model, so clap
/// accepting a value only means it is a real ratio somewhere in the catalog —
/// not that this model renders it. `GET /models` publishes the per-model list
/// as `image.aspect_ratios`, and the API validates against exactly that slice,
/// so checking it here turns a server 400 (arriving after the request is on the
/// wire) into an immediate error that names the model's actual options.
///
/// Best-effort, like the other prechecks: an unreachable catalog or an unknown
/// model falls through to server validation rather than blocking the user.
pub async fn precheck_image_aspect_ratio(
    ctx: &CommandContext,
    model_id: &str,
    ratio: &ImageAspectRatio,
) -> Result<()> {
    let Ok(models) = fetch(ctx).await else {
        return Ok(());
    };
    let Some(model) = models.iter().find(|m| m.id == model_id) else {
        return Ok(());
    };
    let Some(image) = &model.image else {
        return Ok(());
    };
    if image.aspect_ratios.is_empty() {
        anyhow::bail!(
            "--aspect-ratio: {model_id} does not support aspect-ratio selection \
             — pick a model that lists `aspect ratios` in `nolgia models get <model>`"
        );
    }
    if !image.aspect_ratios.iter().any(|r| *r == ratio.to_string()) {
        anyhow::bail!(
            "--aspect-ratio {ratio}: not supported by {model_id}; available: {}",
            image_ratio_list(&image.aspect_ratios)
        );
    }
    Ok(())
}

/// e.g. `16:9, 9:16, 1:1` — in the order the catalog publishes them,
/// including any ratio added to the API after this CLI was built.
fn image_ratio_list(ratios: &[String]) -> String {
    ratios.join(", ")
}

async fn list(args: ListArgs, ctx: &CommandContext) -> Result<()> {
    let mut models = fetch(ctx).await?;
    if let Some(filter) = args.modality {
        models.retain(|m| filter.matches(&m.modality));
    }
    match ctx.format() {
        OutputFormat::Json => print_json(&models),
        OutputFormat::Text => {
            for model in &models {
                let star = if model.recommended { "*" } else { " " };
                println!(
                    "{star} {:52} {:7} {:28} {}",
                    model.id,
                    model.modality.to_lowercase(),
                    cost_line(model),
                    capability_line(model),
                );
            }
            println!("\n* recommended. `nolgia models get <id>` for details.");
            Ok(())
        }
    }
}

async fn get(args: GetArgs, ctx: &CommandContext) -> Result<()> {
    let models = fetch(ctx).await?;
    let model = models.iter().find(|m| m.id == args.id).with_context(|| {
        format!(
            "model {:?} not in the catalog — see `nolgia models list`",
            args.id
        )
    })?;
    match ctx.format() {
        OutputFormat::Json => print_json(model),
        OutputFormat::Text => {
            println!("{}", model.id);
            println!("  modality:    {}", model.modality);
            println!("  recommended: {}", model.recommended);
            println!("  cost:        {}", cost_line(model));
            let caps = capability_line(model);
            if !caps.is_empty() {
                println!("  supports:    {caps}");
            }
            if let Some(quality) = &model.quality {
                let mut label = "quality:    ";
                for line in quality_lines(model, quality) {
                    println!("  {label} {line}");
                    label = "            ";
                }
            }
            if let Some(image) = &model.image
                && !image.aspect_ratios.is_empty()
            {
                println!(
                    "  aspect ratios: {}",
                    image_ratio_list(&image.aspect_ratios)
                );
            }
            if let Some(refs) = &model.references {
                let mut parts: Vec<String> = Vec::new();
                if refs.start_frame {
                    parts.push("start-frame".to_string());
                }
                if refs.end_frame {
                    parts.push("end-frame".to_string());
                }
                if refs.video_refs_max > 0 {
                    parts.push(format!("video-refs <={}", refs.video_refs_max));
                }
                if refs.element_refs_max > 0 {
                    parts.push(format!("elements <={}", refs.element_refs_max));
                }
                if refs.audio_refs_max > 0 {
                    parts.push(format!("audio-refs <={}", refs.audio_refs_max));
                }
                if !refs.bitrate_modes.is_empty() {
                    parts.push(format!("bitrate {}", bitrate_list(&refs.bitrate_modes)));
                }
                if !parts.is_empty() {
                    println!("  references:  {}", parts.join(", "));
                }
            }
            if let Some(audio) = &model.audio
                && !audio.voices.is_empty()
            {
                for voice in &audio.voices {
                    match &voice.label {
                        Some(label) => println!("  voice:       {}  ({label})", voice.id),
                        None => println!("  voice:       {}", voice.id),
                    }
                }
            }
            Ok(())
        }
    }
}
