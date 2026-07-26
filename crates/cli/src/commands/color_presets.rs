//! Built-in color-grade presets ("LUT looks") for Studio compositions.
//! The catalog is embedded in the service — identical for every caller,
//! no auth required — and `colorGrade` fields in compositions reference
//! presets by slug.

use anyhow::{Context, Result};
use clap::{Args, Subcommand};
use futures_util::StreamExt;
use std::{fs, path::PathBuf};

use super::CommandContext;
use crate::output::{OutputFormat, print_json};

#[derive(Subcommand, Debug)]
pub enum ColorPresetsCommand {
    /// List the built-in preset looks (slug, name, description)
    List,
    /// Download a preset's .cube LUT (stdout by default)
    Cube(CubeArgs),
}

#[derive(Args, Debug)]
pub struct CubeArgs {
    /// Preset slug — see `nolgia color-presets list`
    pub slug: String,
    /// Write the .cube to this file instead of stdout
    #[arg(long, short = 'o', value_name = "FILE")]
    pub out: Option<PathBuf>,
}

pub async fn run(command: ColorPresetsCommand, ctx: &CommandContext) -> Result<()> {
    match command {
        ColorPresetsCommand::List => list(ctx).await,
        ColorPresetsCommand::Cube(args) => cube(args, ctx).await,
    }
}

async fn list(ctx: &CommandContext) -> Result<()> {
    let catalog = match ctx.client().list_color_presets().send().await {
        Ok(response) => response.into_inner(),
        Err(err) => return Err(super::api_error(err, "listing color presets").await),
    };
    match ctx.format() {
        OutputFormat::Json => print_json(&catalog),
        OutputFormat::Text => {
            let slug_width = catalog
                .presets
                .iter()
                .map(|p| p.slug.len())
                .max()
                .unwrap_or(0);
            let name_width = catalog
                .presets
                .iter()
                .map(|p| p.name.len())
                .max()
                .unwrap_or(0);
            for preset in &catalog.presets {
                println!(
                    "{:slug_width$}  {:name_width$}  {}",
                    preset.slug, preset.name, preset.description,
                );
            }
            println!("\n`nolgia color-presets cube <slug>` downloads the .cube LUT.");
            Ok(())
        }
    }
}

async fn cube(args: CubeArgs, ctx: &CommandContext) -> Result<()> {
    let response = match ctx
        .client()
        .get_color_preset_cube()
        .slug(args.slug.as_str())
        .send()
        .await
    {
        Ok(response) => response,
        Err(err) => return Err(super::api_error(err, "downloading color-preset cube").await),
    };
    let mut stream = response.into_inner_stream();
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.next().await {
        bytes.extend_from_slice(&chunk.context("reading cube download")?);
    }
    let cube = String::from_utf8(bytes).context("cube payload is not UTF-8 text")?;

    match args.out {
        Some(out) => {
            if let Some(parent) = out.parent() {
                fs::create_dir_all(parent)
                    .with_context(|| format!("creating {}", parent.display()))?;
            }
            fs::write(&out, &cube).with_context(|| format!("writing {}", out.display()))?;
            match ctx.format() {
                OutputFormat::Json => {
                    print_json(&serde_json::json!({"slug": args.slug, "wrote": out}))
                }
                OutputFormat::Text => {
                    println!("wrote {}", out.display());
                    Ok(())
                }
            }
        }
        None => match ctx.format() {
            OutputFormat::Json => print_json(&serde_json::json!({"slug": args.slug, "cube": cube})),
            OutputFormat::Text => {
                print!("{cube}");
                Ok(())
            }
        },
    }
}
