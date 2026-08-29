//! Per-clip timeline masks for Studio compositions — Premiere-style Opacity
//! masks (`rectangle`, `ellipse`, `polygon` with feather, expansion, opacity
//! and inverted controls). A mask is a SPARSE JSON object authored as a
//! `data-mask` attribute on a timed element, or carried by the
//! `nolgia-edits.json` overlay as a `mask` field; every field is optional and
//! an absent field is the default.
//!
//! `validate` runs the platform's sanitizer (`POST /masks:validate`, public,
//! stateless) and prints the canonical mask the renderer would actually draw
//! plus one line per field it clamped, dropped, or could not draw — so an
//! authored mask can be linted before a push instead of rendering differently
//! from the author's intent. `example` prints a contract-true starter mask
//! for each shape without touching the network.

use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand, ValueEnum};
use serde::Serialize;
use std::{
    fs,
    io::{self, Read},
};

use super::CommandContext;
use crate::output::{OutputFormat, print_json};
use nolgia_client::types::{Mask, MaskProblem, MaskValidateRequest};

#[derive(Subcommand, Debug)]
pub enum MasksCommand {
    /// Run the mask sanitizer and print what the renderer would draw, plus
    /// every clamp/drop diagnostic
    Validate(ValidateArgs),
    /// Print a contract-true starter mask for a shape (offline, no login)
    Example(ExampleArgs),
}

#[derive(Args, Debug)]
pub struct ValidateArgs {
    /// The candidate mask: inline JSON, `@path/to/mask.json`, or `-` for stdin.
    /// Any JSON value is accepted so junk can be diagnosed.
    #[arg(value_name = "MASK")]
    pub mask: String,
    /// Exit 1 when the sanitizer reports any problem — a lint gate: a mask that
    /// produces problems renders differently from what was authored
    #[arg(long)]
    pub strict: bool,
}

#[derive(Args, Debug)]
pub struct ExampleArgs {
    /// Which starter to print
    #[arg(value_enum)]
    pub shape: ExampleShape,
}

#[derive(ValueEnum, Clone, Copy, Debug)]
pub enum ExampleShape {
    /// A rounded picture-in-picture window in the top-right corner
    Rectangle,
    /// A soft vignette-style oval
    Ellipse,
    /// A triangular pen-path cutout
    Polygon,
}

pub async fn run(command: MasksCommand, ctx: &CommandContext) -> Result<()> {
    match command {
        MasksCommand::Validate(args) => validate(args, ctx).await,
        MasksCommand::Example(args) => example(args, ctx.format()),
    }
}

async fn validate(args: ValidateArgs, ctx: &CommandContext) -> Result<()> {
    let candidate = read_candidate(&args.mask)?;
    let body = MaskValidateRequest { mask: candidate };
    let validation = match ctx.client().validate_mask().body(body).send().await {
        Ok(response) => response.into_inner(),
        Err(err) => return Err(super::api_error(err, "validating mask").await),
    };

    match ctx.format() {
        OutputFormat::Json => print_json(&Verdict {
            mask: validation.mask.as_ref().map(CanonicalMask::from),
            identity: validation.identity,
            problems: &validation.problems,
        })?,
        OutputFormat::Text => {
            match &validation.mask {
                Some(mask) => println!(
                    "{}",
                    serde_json::to_string_pretty(&CanonicalMask::from(mask))?
                ),
                None if validation.identity => {
                    println!("null (identity — clears an authored mask)")
                }
                None => println!("null (not a drawable mask)"),
            }
            for problem in &validation.problems {
                println!("  {}: {}", display_path(&problem.path), problem.message);
            }
        }
    }

    if args.strict && !validation.problems.is_empty() {
        bail!(
            "mask has {} problem{} (--strict)",
            validation.problems.len(),
            if validation.problems.len() == 1 {
                ""
            } else {
                "s"
            }
        );
    }
    Ok(())
}

/// Resolve the `<MASK>` argument to the JSON value to sanitize: `-` reads
/// stdin, `@path` reads a file, anything else is the JSON itself.
fn read_candidate(arg: &str) -> Result<serde_json::Value> {
    let (text, source) = if arg == "-" {
        let mut text = String::new();
        io::stdin()
            .read_to_string(&mut text)
            .context("reading mask from stdin")?;
        (text, "stdin".to_string())
    } else if let Some(path) = arg.strip_prefix('@') {
        let text = fs::read_to_string(path).with_context(|| format!("reading {path}"))?;
        (text, path.to_string())
    } else {
        (arg.to_string(), "<MASK>".to_string())
    };
    serde_json::from_str(&text).with_context(|| format!("parsing mask JSON from {source}"))
}

/// A problem's `path` is `""` when it is about the mask as a whole (not an
/// object, undrawable shape, too few vertices); name that rather than print
/// a bare colon.
fn display_path(path: &str) -> &str {
    if path.is_empty() { "(mask)" } else { path }
}

/// The sanitizer's verdict, re-serialized with the mask in its canonical
/// form (see [`CanonicalMask`]) and the fields in spec order.
#[derive(Serialize)]
struct Verdict<'a> {
    mask: Option<CanonicalMask>,
    identity: bool,
    problems: &'a [MaskProblem],
}

/// The contract's canonical serialization of a sparse mask: fields in
/// contract-table order (`shape, x, y, width, height, rotation, cornerRadius,
/// points, feather, expansion, opacity, inverted`) and numbers as their
/// shortest decimal (`50`, not `50.0`). This is the exact text an agent
/// pastes into a `data-mask` attribute or an overlay `mask` field, so it
/// should look like the contract's own examples rather than like serde's
/// alphabetical, float-suffixed rendering of the generated type.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CanonicalMask {
    #[serde(skip_serializing_if = "Option::is_none")]
    shape: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    x: Option<serde_json::Number>,
    #[serde(skip_serializing_if = "Option::is_none")]
    y: Option<serde_json::Number>,
    #[serde(skip_serializing_if = "Option::is_none")]
    width: Option<serde_json::Number>,
    #[serde(skip_serializing_if = "Option::is_none")]
    height: Option<serde_json::Number>,
    #[serde(skip_serializing_if = "Option::is_none")]
    rotation: Option<serde_json::Number>,
    #[serde(skip_serializing_if = "Option::is_none")]
    corner_radius: Option<serde_json::Number>,
    #[serde(skip_serializing_if = "Option::is_none")]
    points: Option<Vec<[serde_json::Number; 2]>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    feather: Option<serde_json::Number>,
    #[serde(skip_serializing_if = "Option::is_none")]
    expansion: Option<serde_json::Number>,
    #[serde(skip_serializing_if = "Option::is_none")]
    opacity: Option<serde_json::Number>,
    #[serde(skip_serializing_if = "Option::is_none")]
    inverted: Option<bool>,
}

impl From<&Mask> for CanonicalMask {
    fn from(mask: &Mask) -> Self {
        let points = (!mask.points.is_empty()).then(|| {
            mask.points
                .iter()
                .map(|point| [number(point[0]), number(point[1])])
                .collect()
        });
        Self {
            shape: mask.shape.clone(),
            x: mask.x.map(number),
            y: mask.y.map(number),
            width: mask.width.map(number),
            height: mask.height.map(number),
            rotation: mask.rotation.map(number),
            corner_radius: mask.corner_radius.map(number),
            points,
            feather: mask.feather.map(number),
            expansion: mask.expansion.map(number),
            opacity: mask.opacity.map(number),
            inverted: mask.inverted,
        }
    }
}

/// Shortest round-trip rendering of a sanitized number: whole values print
/// as integers (`50`), everything else as its shortest decimal (`12.5`). The
/// sanitizer rounds to two decimals, so nothing longer ever arrives.
fn number(value: f64) -> serde_json::Number {
    if value.fract() == 0.0 && value.abs() < 1e15 {
        serde_json::Number::from(value as i64)
    } else {
        // `None` only for NaN/±inf, which the sanitizer never emits; fall
        // back to zero rather than panic on a value we cannot represent.
        serde_json::Number::from_f64(value).unwrap_or_else(|| serde_json::Number::from(0))
    }
}

/// A rounded picture-in-picture window in the top-right corner.
const RECTANGLE_EXAMPLE: &str = r#"{
  "x": 75,
  "y": 22,
  "width": 40,
  "height": 24,
  "cornerRadius": 24,
  "feather": 2
}"#;

/// A soft oval — the classic vignette / spotlight reveal.
const ELLIPSE_EXAMPLE: &str = r#"{
  "shape": "ellipse",
  "y": 40,
  "width": 60,
  "height": 45,
  "feather": 30
}"#;

/// A triangular pen-path cutout; `points` are `[x, y]` percentages of the
/// clip box in draw order (the path closes itself).
const POLYGON_EXAMPLE: &str = r#"{
  "shape": "polygon",
  "points": [[0, 0], [100, 0], [50, 100]],
  "feather": 8
}"#;

/// Print a starter mask. Runs before any client is built: nothing here needs
/// a login or a request.
pub fn example(args: ExampleArgs, format: OutputFormat) -> Result<()> {
    let mask = match args.shape {
        ExampleShape::Rectangle => RECTANGLE_EXAMPLE,
        ExampleShape::Ellipse => ELLIPSE_EXAMPLE,
        ExampleShape::Polygon => POLYGON_EXAMPLE,
    };
    println!("{mask}");
    if format == OutputFormat::Text {
        println!(
            "\nCoordinates are % of the clip box; feather/expansion/cornerRadius are native \
             canvas px. Paste it into a `data-mask` attribute or an overlay `mask` field, \
             and check any edit with `nolgia masks validate '<json>'` (or `@file`, `-`). \
             Full contract: the `Mask` schema in https://docs.nolgia.ai/api/openapi.yaml"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn examples_are_valid_canonical_json() {
        for text in [RECTANGLE_EXAMPLE, ELLIPSE_EXAMPLE, POLYGON_EXAMPLE] {
            serde_json::from_str::<serde_json::Value>(text).expect("valid JSON");
        }

        let rectangle: serde_json::Value =
            serde_json::from_str(RECTANGLE_EXAMPLE).expect("valid JSON");
        let ellipse: serde_json::Value = serde_json::from_str(ELLIPSE_EXAMPLE).expect("valid JSON");
        assert!(rectangle.get("shape").is_none());
        assert!(ellipse.get("x").is_none());
    }

    #[test]
    fn canonical_mask_keeps_contract_order_and_integer_rendering() {
        let mask = Mask {
            shape: Some("rectangle".into()),
            x: Some(75.0),
            y: Some(22.5),
            width: Some(40.0),
            height: Some(24.0),
            rotation: None,
            corner_radius: Some(24.0),
            points: Vec::new(),
            feather: Some(2.0),
            expansion: None,
            opacity: None,
            inverted: Some(true),
        };
        let text = serde_json::to_string(&CanonicalMask::from(&mask)).unwrap();
        assert_eq!(
            text,
            r#"{"shape":"rectangle","x":75,"y":22.5,"width":40,"height":24,"cornerRadius":24,"feather":2,"inverted":true}"#
        );
    }

    #[test]
    fn canonical_mask_renders_polygon_points_as_pairs() {
        let mask = Mask {
            shape: Some("polygon".into()),
            points: vec![
                nolgia_client::types::MaskPoint([0.0, 0.0]),
                nolgia_client::types::MaskPoint([100.0, 0.0]),
                nolgia_client::types::MaskPoint([50.0, 100.0]),
            ],
            ..Default::default()
        };
        let text = serde_json::to_string(&CanonicalMask::from(&mask)).unwrap();
        assert_eq!(
            text,
            r#"{"shape":"polygon","points":[[0,0],[100,0],[50,100]]}"#
        );
    }

    #[test]
    fn empty_problem_path_names_the_mask() {
        assert_eq!(display_path(""), "(mask)");
        assert_eq!(display_path("points[2]"), "points[2]");
    }
}
