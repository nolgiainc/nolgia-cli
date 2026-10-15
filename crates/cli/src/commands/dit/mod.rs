//! `nolgia dit`: data wrangling for camera cards, on this computer, with no
//! login and no upload. Copy cards to one to three drives and prove every
//! copy (`offload`), re-check a copy later (`verify`), write the industry
//! manifest for a folder (`manifest`), list what was shot (`inventory`),
//! name the files by shoot, date, time and camera (`rename`, `undo`) and
//! write the DIT report (`report`).
//!
//! Exit codes: 0 when everything checked out, 68 when a copy or a file does
//! not match its source or manifest (or is missing), 130 when interrupted
//! (run the same command again to resume), 1 for any other error.

mod files;
mod hash;
mod manifest;
mod meta;
mod offload;
mod rename;
mod report;
mod verify;

use std::{
    fmt,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use anyhow::{Result, bail};
use clap::{Args, Subcommand};

use crate::output::{OutputContext, OutputFormat, print_json};

/// A copy or file that does not match its source or manifest.
pub const EXIT_MISMATCH: u8 = 68;
/// Stopped by Ctrl-C or a cancel; the same command resumes.
pub const EXIT_INTERRUPTED: u8 = 130;

/// The command finished and printed its result, but the result is a
/// failure the caller must see in the exit status.
#[derive(Debug)]
pub struct DitExit(pub u8);

impl fmt::Display for DitExit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "nolgia dit finished with exit status {}", self.0)
    }
}

impl std::error::Error for DitExit {}

#[derive(Subcommand, Debug)]
pub enum DitCommand {
    /// Copy one or more camera cards to one to three drives, read every copy
    /// back to prove it matches, and write ASC MHL and TSV manifests. Never
    /// changes the card and never replaces a file at a destination; run the
    /// same command again to resume an interrupted offload
    Offload(OffloadArgs),
    /// Re-read every file a manifest lists (ASC MHL or TSV) and report any
    /// that differ or are missing. Read-only unless --record
    Verify(VerifyArgs),
    /// Hash a folder in place and write its ASC MHL generation and a TSV
    /// (for footage that was copied some other way)
    Manifest(ManifestArgs),
    /// List the media in a folder by camera: dates, durations, frame rates,
    /// resolution, gamma (S-Log3 ...), lenses. Flags camera clocks that look off
    Inventory(InventoryArgs),
    /// Name media files by a pattern (shoot, date, time, camera, original
    /// name), with a lookup sheet and undo. A dry run unless --apply
    Rename(RenameArgs),
    /// Restore the original names from a rename's lookup sheet
    Undo(UndoArgs),
    /// Write the DIT report (Markdown and JSON) for a folder
    Report(ReportArgs),
    /// Show the progress of a running or finished offload at a destination
    Status(StatusArgs),
}

#[derive(Args, Debug)]
pub struct OffloadArgs {
    /// Cards or folders to copy (a mounted card like E:\ or /Volumes/A001)
    #[arg(value_name = "CARD", required = true)]
    pub sources: Vec<PathBuf>,
    /// A destination drive or folder; repeat for a second and third copy
    #[arg(long = "to", value_name = "DEST", required = true)]
    pub dests: Vec<PathBuf>,
    /// Shoot folder to create at each destination (cards go inside it)
    #[arg(long, value_name = "NAME")]
    pub shoot: Option<String>,
    /// Name for each card's folder, in the order the cards are given
    /// (default: the card's own folder or volume name)
    #[arg(long = "card", value_name = "NAME")]
    pub cards: Vec<String>,
    /// Hashes to compute, comma-separated: xxh64 (always), md5, sha1
    #[arg(long, default_value = "xxh64", value_name = "LIST")]
    pub hash: String,
    /// Show what would be copied, where, and any conflicts; copy nothing
    #[arg(long)]
    pub dry_run: bool,
    /// Cards to read at the same time (1-4). Files on one card are always
    /// read one after another
    #[arg(long, default_value_t = 2, value_name = "N")]
    pub max_parallel: usize,
}

#[derive(Args, Debug)]
pub struct VerifyArgs {
    /// The folder to check (the copied card or shoot folder)
    pub dir: PathBuf,
    /// Manifest to check against: a .tsv, an .mhl, or an ascmhl folder
    /// (default: DIR/ascmhl, else DIR/_DIT/manifest.tsv)
    #[arg(long, value_name = "PATH")]
    pub manifest: Option<PathBuf>,
    /// Only check files under this subfolder (repeatable)
    #[arg(long, value_name = "SUBFOLDER")]
    pub only: Vec<String>,
    /// Append the result to DIR/ascmhl as a new generation (writes to DIR)
    #[arg(long)]
    pub record: bool,
}

#[derive(Args, Debug)]
pub struct ManifestArgs {
    /// The folder to hash in place
    pub dir: PathBuf,
    /// Hashes to compute, comma-separated: xxh64 (always), md5, sha1
    #[arg(long, default_value = "xxh64", value_name = "LIST")]
    pub hash: String,
}

#[derive(Args, Debug)]
pub struct InventoryArgs {
    /// The folder (card, copy or shoot folder) to list
    pub dir: PathBuf,
    /// Correct a camera's clock, by model: "ILME-FX3A=-12h" (repeatable)
    #[arg(long = "clock-offset", value_name = "MODEL=OFFSET")]
    pub offsets: Vec<String>,
    /// Also write every file's details to this CSV
    #[arg(long, value_name = "PATH")]
    pub csv: Option<PathBuf>,
    /// List every file in the output, not only the per-camera summary
    #[arg(long)]
    pub files: bool,
}

#[derive(Args, Debug)]
pub struct RenameArgs {
    /// The folder of media to rename (renamed in place)
    pub dir: PathBuf,
    /// Name pattern; tokens: {shoot} {date} {time} {camera} {orig}. {orig}
    /// (the camera's own name) is required
    #[arg(
        long,
        default_value = "{shoot}_{date}_{time}_{camera}_{orig}",
        value_name = "PATTERN"
    )]
    pub pattern: String,
    /// Shoot name for {shoot}
    #[arg(long, value_name = "NAME")]
    pub shoot: Option<String>,
    /// Label a camera for {camera}, by model: "ILME-FX3A=A-FX3A" (repeatable;
    /// default A-<model>, B-<model> ...)
    #[arg(long = "camera", value_name = "MODEL=LABEL")]
    pub cameras: Vec<String>,
    /// Correct a camera's clock, by model: "ILME-FX3A=-12h" (repeatable)
    #[arg(long = "clock-offset", value_name = "MODEL=OFFSET")]
    pub offsets: Vec<String>,
    /// Lookup sheet to read and append to (default DIR/_DIT/rename-map.csv)
    #[arg(long, value_name = "PATH")]
    pub map: Option<PathBuf>,
    /// Rename for real (default is a dry run that changes nothing)
    #[arg(long)]
    pub apply: bool,
}

#[derive(Args, Debug)]
pub struct UndoArgs {
    /// The folder that was renamed
    pub dir: PathBuf,
    /// Lookup sheet to undo (default DIR/_DIT/rename-map.csv)
    #[arg(long, value_name = "PATH")]
    pub map: Option<PathBuf>,
}

#[derive(Args, Debug)]
pub struct ReportArgs {
    /// The folder to report on
    pub dir: PathBuf,
    /// Manifest to verify against (implies --verify)
    #[arg(long, value_name = "PATH")]
    pub manifest: Option<PathBuf>,
    /// Re-read every file against the manifest for the report
    #[arg(long)]
    pub verify: bool,
    /// Only verify files under this subfolder (repeatable)
    #[arg(long, value_name = "SUBFOLDER")]
    pub only: Vec<String>,
    /// Correct a camera's clock, by model: "ILME-FX3A=-12h" (repeatable)
    #[arg(long = "clock-offset", value_name = "MODEL=OFFSET")]
    pub offsets: Vec<String>,
    /// Where to write the report (.md; a .json is written beside it).
    /// Default DIR/_DIT/DIT-REPORT-<time>.md
    #[arg(long, value_name = "PATH")]
    pub out: Option<PathBuf>,
}

#[derive(Args, Debug)]
pub struct StatusArgs {
    /// The destination (or its shoot folder) an offload is writing to
    pub dir: PathBuf,
}

/// Run the blocking work on a worker thread while Ctrl-C sets the stop flag.
async fn with_stop<T: Send + 'static>(
    work: impl FnOnce(Arc<AtomicBool>) -> Result<T> + Send + 'static,
) -> Result<T> {
    let stop = Arc::new(AtomicBool::new(false));
    let s = stop.clone();
    let watcher = tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            eprintln!("Stopping at the next safe point; run the same command again to resume.");
            s.store(true, Ordering::Relaxed);
        }
    });
    let result = tokio::task::spawn_blocking(move || work(stop)).await?;
    watcher.abort();
    result
}

fn emit<T: serde::Serialize>(
    output: &OutputContext,
    value: &T,
    text: impl FnOnce() -> String,
) -> Result<()> {
    if output.format() == OutputFormat::Json || output.is_selected() {
        print_json(output, value)
    } else {
        println!("{}", text());
        Ok(())
    }
}

pub async fn run(command: DitCommand, output: &OutputContext) -> Result<()> {
    match command {
        DitCommand::Offload(a) => offload_cmd(a, output).await,
        DitCommand::Verify(a) => verify_cmd(a, output).await,
        DitCommand::Manifest(a) => manifest_cmd(a, output).await,
        DitCommand::Inventory(a) => inventory_cmd(a, output).await,
        DitCommand::Rename(a) => rename_cmd(a, output).await,
        DitCommand::Undo(a) => undo_cmd(a, output).await,
        DitCommand::Report(a) => report_cmd(a, output).await,
        DitCommand::Status(a) => status_cmd(a, output),
    }
}

async fn offload_cmd(a: OffloadArgs, output: &OutputContext) -> Result<()> {
    let opts = offload::Options {
        sources: a.sources,
        cards: a.cards,
        dests: a.dests,
        shoot: a.shoot,
        algos: hash::Algo::parse_list(&a.hash)?,
        dry_run: a.dry_run,
        max_parallel: a.max_parallel,
    };
    let (plan, cards) = offload::plan(&opts)?;
    if opts.dry_run || !plan.conflicts.is_empty() {
        let conflicted = !plan.conflicts.is_empty();
        emit(output, &plan, || {
            let mut s = format!(
                "Offload plan: {} files, {} from {} card(s) to {} destination(s)\n",
                plan.files,
                offload::human_bytes(plan.bytes),
                plan.cards.len(),
                plan.destinations.len()
            );
            for c in &plan.cards {
                s.push_str(&format!(
                    "  card {} <- {} ({} files, {})\n",
                    c.card,
                    c.source,
                    c.files,
                    offload::human_bytes(c.bytes)
                ));
            }
            for d in &plan.destinations {
                s.push_str(&format!(
                    "  to {} ({} free, {} files already there at the same size)\n",
                    d.folder,
                    d.free_bytes
                        .map(offload::human_bytes)
                        .unwrap_or_else(|| "unknown".into()),
                    d.already_there_same_size
                ));
            }
            for c in &plan.conflicts {
                s.push_str(&format!("  CONFLICT {}: {}\n", c.path, c.reason));
            }
            if conflicted {
                s.push_str("Nothing was copied: resolve the conflicts (nothing at a destination is ever replaced).");
            } else {
                s.push_str("Dry run: nothing was copied.");
            }
            s
        })?;
        if conflicted {
            return Err(DitExit(EXIT_MISMATCH).into());
        }
        return Ok(());
    }
    let mut outcome = with_stop(move |stop| offload::run(&opts, stop, plan, cards)).await?;
    // The report for each destination's shoot folder, from the copies.
    let status = outcome.status.clone();
    if status != "interrupted"
        && let Some(r) = outcome.results.first()
    {
        let folder = PathBuf::from(&r.folder);
        let shoot_dir = folder.parent().map(PathBuf::from).unwrap_or(folder.clone());
        let out = offload::dit_dir(&shoot_dir, None).join(format!(
            "DIT-REPORT-{}.md",
            chrono::Local::now().format("%Y%m%d-%H%M%S")
        ));
        let note = format!(
            "Every copy was read back from its destination and matched its card when it was \
             written: {} file copies verified ({}), {} failed. ASC MHL and TSV manifests list each card.",
            outcome.files_verified,
            outcome.plan.hashes.join(", "),
            outcome
                .results
                .iter()
                .map(|r| r.failed.len())
                .sum::<usize>()
        );
        let ro = report::ReportOptions {
            offload_note: Some(note),
            dir: shoot_dir,
            manifest: None,
            verify: false,
            only: Vec::new(),
            offsets: Vec::new(),
            out: Some(out),
        };
        if let Ok(rep) = report::report(&ro, Arc::new(AtomicBool::new(false))) {
            outcome.report = Some(rep.markdown);
        }
    }
    emit(output, &outcome, || {
        let mut s = format!(
            "Offload {}: {} file copies verified in {:.0} s ({} MB/s)\n",
            outcome.status, outcome.files_verified, outcome.seconds, outcome.mb_per_s
        );
        for r in &outcome.results {
            s.push_str(&format!(
                "  {} -> {}: {} copied and verified, {} already there, {} resumed, {} failed\n",
                r.card,
                r.folder,
                r.verified,
                r.already_there,
                r.resumed,
                r.failed.len()
            ));
            for f in &r.failed {
                s.push_str(&format!("    FAILED {}: {}\n", f.path, f.reason));
            }
        }
        if let Some(rep) = &outcome.report {
            s.push_str(&format!("Report: {rep}\n"));
        }
        if let Some(h) = &outcome.resume_hint {
            s.push_str(&format!("Interrupted: {h}.\n"));
        }
        s.trim_end().to_string()
    })?;
    match status.as_str() {
        "complete" => Ok(()),
        "interrupted" => Err(DitExit(EXIT_INTERRUPTED).into()),
        _ => Err(DitExit(EXIT_MISMATCH).into()),
    }
}

async fn verify_cmd(a: VerifyArgs, output: &OutputContext) -> Result<()> {
    let opts = verify::Options {
        dir: a.dir,
        manifest: a.manifest,
        only: a.only,
        record: a.record,
    };
    let out = with_stop(move |stop| verify::run(&opts, stop)).await?;
    let status = out.status.clone();
    emit(output, &out, || {
        let mut s = format!(
            "Verify {}: {} of {} files match ({}, {}), {} differ, {} missing; {} files on disk are not in the manifest\nManifest: {}",
            out.status,
            out.files_ok,
            out.files_in_manifest,
            out.hash,
            offload::human_bytes(out.bytes_checked),
            out.mismatched.len(),
            out.missing.len(),
            out.not_in_manifest.len(),
            out.manifest
        );
        for p in out.mismatched.iter().chain(&out.missing) {
            s.push_str(&format!("\n  {}: {}", p.path, p.detail));
        }
        for w in &out.warnings {
            s.push_str(&format!("\n  warning: {w}"));
        }
        s
    })?;
    match status.as_str() {
        "ok" => Ok(()),
        "interrupted" => Err(DitExit(EXIT_INTERRUPTED).into()),
        _ => Err(DitExit(EXIT_MISMATCH).into()),
    }
}

async fn manifest_cmd(a: ManifestArgs, output: &OutputContext) -> Result<()> {
    let algos = hash::Algo::parse_list(&a.hash)?;
    let dir = a.dir;
    if !dir.is_dir() {
        bail!("{} is not a folder", dir.display());
    }
    let (mhl, tsv, count) = with_stop(move |stop| -> Result<(PathBuf, PathBuf, usize)> {
        let walk = files::walk(&dir)?;
        let mut records = Vec::new();
        for f in &walk.files {
            if stop.load(Ordering::Relaxed) {
                bail!(DitExit(EXIT_INTERRUPTED));
            }
            let (size, hashes) = hash::hash_file(&f.abs, &algos)?;
            records.push(manifest::Record {
                rel: f.rel.clone(),
                size,
                mtime: f.mtime,
                hashes,
            });
        }
        let mhl = manifest::write_mhl_generation(
            &dir,
            &records,
            &walk.empty_dirs,
            manifest::Process::InPlace,
            "original",
        )?;
        let tsv = manifest::tsv_path_for(&dir);
        manifest::write_tsv(&tsv, &dir.display().to_string(), &records, &algos)?;
        Ok((mhl, tsv, records.len()))
    })
    .await?;
    let value = serde_json::json!({"files": count, "mhl": mhl.display().to_string(), "tsv": tsv.display().to_string()});
    emit(output, &value, || {
        format!(
            "Hashed {count} files.\nASC MHL: {}\nTSV: {}",
            mhl.display(),
            tsv.display()
        )
    })
}

async fn inventory_cmd(a: InventoryArgs, output: &OutputContext) -> Result<()> {
    let list_files = a.files;
    let mut inv = tokio::task::spawn_blocking(move || {
        report::inventory(&a.dir, &a.offsets, a.csv.as_deref())
    })
    .await??;
    if !list_files {
        inv.items.clear();
    }
    emit(output, &inv, || {
        let mut s = format!(
            "{}: {} media files, {} ({} card-structure files, {} other)\n",
            inv.folder,
            inv.media_files,
            offload::human_bytes(inv.bytes),
            inv.card_files,
            inv.other_files
        );
        for c in &inv.cameras {
            s.push_str(&format!(
                "  {}: {} videos, {} photos, {}, {} to {}{}\n",
                c.camera,
                c.videos,
                c.photos,
                offload::human_bytes(c.bytes),
                c.first.clone().unwrap_or_default(),
                c.last.clone().unwrap_or_default(),
                if c.gamma.is_empty() {
                    String::new()
                } else {
                    format!(
                        ", {}",
                        c.gamma.iter().cloned().collect::<Vec<_>>().join("/")
                    )
                }
            ));
        }
        for w in &inv.warnings {
            s.push_str(&format!("  warning: {w}\n"));
        }
        if let Some(c) = &inv.csv {
            s.push_str(&format!("CSV: {c}\n"));
        }
        s.trim_end().to_string()
    })
}

async fn rename_cmd(a: RenameArgs, output: &OutputContext) -> Result<()> {
    let opts = rename::Options {
        dir: a.dir,
        pattern: a.pattern,
        shoot: a.shoot,
        cameras: a.cameras,
        offsets: a.offsets,
        map: a.map,
        apply: a.apply,
    };
    let out = with_stop(move |stop| {
        let (outcome, moves) = rename::plan(&opts)?;
        if opts.apply {
            rename::apply(&opts, outcome, moves, stop)
        } else {
            Ok(outcome)
        }
    })
    .await?;
    let status = out.status.clone();
    emit(output, &out, || {
        let mut s = format!(
            "Rename {}: {} files to rename, {} already named, {} card-structure files kept as they are\n",
            out.status, out.to_rename, out.already_named, out.kept_card_files
        );
        for (cam, label) in &out.cameras {
            s.push_str(&format!("  camera {cam} -> {label}\n"));
        }
        for c in out.changes.iter().take(20) {
            s.push_str(&format!("  {} -> {}\n", c.from, c.to));
        }
        if out.changes.len() > 20 {
            s.push_str(&format!(
                "  ... and {} more (--json lists all)\n",
                out.changes.len() - 20
            ));
        }
        if out.status == "dry-run" {
            s.push_str("Dry run: nothing was renamed. Add --apply to rename.\n");
        }
        if let Some(m) = &out.map {
            s.push_str(&format!(
                "Lookup sheet: {m}\nUndo: {}\n",
                out.undo.clone().unwrap_or_default()
            ));
        }
        for w in &out.warnings {
            s.push_str(&format!("  warning: {w}\n"));
        }
        s.trim_end().to_string()
    })?;
    if status == "interrupted" {
        return Err(DitExit(EXIT_INTERRUPTED).into());
    }
    Ok(())
}

async fn undo_cmd(a: UndoArgs, output: &OutputContext) -> Result<()> {
    let out = with_stop(move |stop| rename::undo(&a.dir, a.map.as_deref(), stop)).await?;
    let status = out.status.clone();
    emit(output, &out, || {
        let mut s = format!(
            "Undo {}: {} files back to their original names\n",
            out.status, out.restored
        );
        for c in &out.skipped {
            s.push_str(&format!("  skipped {} -> {}\n", c.from, c.to));
        }
        s.trim_end().to_string()
    })?;
    if status == "interrupted" {
        return Err(DitExit(EXIT_INTERRUPTED).into());
    }
    Ok(())
}

async fn report_cmd(a: ReportArgs, output: &OutputContext) -> Result<()> {
    let opts = report::ReportOptions {
        offload_note: None,
        dir: a.dir,
        verify: a.verify || a.manifest.is_some(),
        manifest: a.manifest,
        only: a.only,
        offsets: a.offsets,
        out: a.out,
    };
    let rep = with_stop(move |stop| report::report(&opts, stop)).await?;
    let failed = rep
        .verification
        .as_ref()
        .is_some_and(|v| v.status == "failed");
    emit(output, &rep, || {
        format!("DIT report written:\n  {}\n  {}", rep.markdown, rep.json)
    })?;
    if failed {
        return Err(DitExit(EXIT_MISMATCH).into());
    }
    Ok(())
}

fn status_cmd(a: StatusArgs, output: &OutputContext) -> Result<()> {
    let candidates = [
        a.dir.join(files::DIT_DIR).join("offload-progress.json"),
        a.dir.join("offload-progress.json"),
    ];
    let Some(path) = candidates.iter().find(|p| p.is_file()) else {
        bail!(
            "no offload progress at {} (look in the shoot folder the offload writes to)",
            a.dir.display()
        );
    };
    let value: serde_json::Value = serde_json::from_slice(&std::fs::read(path)?)?;
    emit(output, &value, || {
        format!(
            "{} of {} files, {} of {}{}",
            value["files_done"],
            value["files_total"],
            offload::human_bytes(value["bytes_done"].as_u64().unwrap_or(0)),
            offload::human_bytes(value["bytes_total"].as_u64().unwrap_or(0)),
            if value["running"].as_bool() == Some(true) {
                ", running"
            } else {
                ", stopped"
            }
        )
    })
}
