use anyhow::Result;
use clap::{Args, Subcommand};
use serde::Serialize;

use super::{CommandContext, models};
use crate::output::{OutputFormat, print_json};

#[derive(Subcommand, Debug)]
pub enum VoicesCommand {
    /// List the live voice catalog for text-to-speech models
    List(ListArgs),
}

#[derive(Args, Debug)]
pub struct ListArgs {
    /// Only this model's voices, e.g. `fal-ai/elevenlabs/tts/eleven-v3`. The
    /// voice id is what `nolgia gen audio --voice` takes.
    #[arg(long)]
    pub model: Option<String>,
}

#[derive(Serialize)]
struct CatalogVoice<'a> {
    model: &'a str,
    id: &'a str,
    label: Option<&'a str>,
}

pub async fn run(command: VoicesCommand, ctx: &CommandContext) -> Result<()> {
    match command {
        VoicesCommand::List(args) => list(args, ctx).await,
    }
}

async fn list(args: ListArgs, ctx: &CommandContext) -> Result<()> {
    let catalog = models::fetch(ctx).await?;
    if let Some(id) = &args.model {
        let Some(model) = catalog.iter().find(|model| model.id == *id) else {
            let available: Vec<&str> = catalog
                .iter()
                .filter(|model| {
                    model
                        .audio
                        .as_ref()
                        .is_some_and(|audio| !audio.voices.is_empty())
                })
                .map(|model| model.id.as_str())
                .collect();
            anyhow::bail!(
                "unknown model {id}; audio models with voices: {}",
                available.join(", ")
            );
        };
        if !model
            .audio
            .as_ref()
            .is_some_and(|audio| !audio.voices.is_empty())
        {
            anyhow::bail!("{id} publishes no voice catalog (see nolgia models get {id})");
        }
    }
    let mut voices = Vec::new();
    for model in &catalog {
        if args.model.as_ref().is_some_and(|id| *id != model.id) {
            continue;
        }
        if let Some(audio) = &model.audio {
            for voice in &audio.voices {
                voices.push(CatalogVoice {
                    model: &model.id,
                    id: &voice.id,
                    label: voice.label.as_deref(),
                });
            }
        }
    }
    match ctx.format() {
        OutputFormat::Json => print_json(&voices),
        OutputFormat::Text => {
            if voices.is_empty() {
                println!("no voices");
            }
            for voice in voices {
                match voice.label {
                    Some(label) => println!("{}  {}  {label}", voice.model, voice.id),
                    None => println!("{}  {}", voice.model, voice.id),
                }
            }
            Ok(())
        }
    }
}
