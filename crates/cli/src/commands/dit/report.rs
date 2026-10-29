//! `nolgia dit inventory` and `nolgia dit report`.

use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    fs,
    path::{Path, PathBuf},
    sync::{Arc, atomic::AtomicBool},
};

use anyhow::{Context, Result};
use chrono::NaiveDateTime;
use serde::Serialize;

use super::{
    files,
    meta::{self, Kind, Tools},
    offload::{human_bytes, seconds_text},
    rename::{self, is_card_structure},
    verify,
};

#[derive(Serialize, Debug, Clone)]
pub struct Item {
    pub path: String,
    pub kind: Kind,
    pub camera: String,
    pub bytes: u64,
    pub captured: Option<String>,
    /// The capture time after `--clock-offset`, `YYYY-MM-DD HH:MM:SS`.
    pub corrected: Option<String>,
    #[serde(flatten)]
    pub meta: meta::Meta,
}

#[derive(Serialize, Debug, Default)]
pub struct CameraSummary {
    pub camera: String,
    pub clock_offset_minutes: i64,
    pub videos: usize,
    pub photos: usize,
    pub audio: usize,
    pub bytes: u64,
    pub first: Option<String>,
    pub last: Option<String>,
    pub video_seconds: f64,
    pub frame_rates: BTreeSet<String>,
    pub resolutions: BTreeSet<String>,
    pub codecs: BTreeSet<String>,
    pub gamma: BTreeSet<String>,
    pub lenses: BTreeSet<String>,
    /// Files whose time is the file's modified time: the file itself
    /// records no capture time we can read.
    pub times_from_file: usize,
}

#[derive(Serialize, Debug)]
pub struct Inventory {
    pub folder: String,
    pub media_files: usize,
    pub other_files: usize,
    pub card_files: usize,
    pub bytes: u64,
    pub cameras: Vec<CameraSummary>,
    pub warnings: Vec<String>,
    pub items: Vec<Item>,
    pub csv: Option<String>,
}

fn fmt_dt(t: NaiveDateTime) -> String {
    t.format("%Y-%m-%d %H:%M:%S").to_string()
}

pub fn inventory(dir: &Path, offsets_raw: &[String], csv_out: Option<&Path>) -> Result<Inventory> {
    let offsets = rename::parse_offsets(offsets_raw)?;
    let tools = Tools::detect();
    let walk = files::walk(dir)?;
    let mut items = Vec::new();
    let (mut other, mut card) = (0usize, 0usize);
    let mut warnings = Vec::new();
    let mut zero = Vec::new();
    for f in &walk.files {
        if is_card_structure(&f.rel) {
            card += 1;
            continue;
        }
        let kind = meta::kind_of(&f.abs);
        // Sony sidecars and other non-media are counted, not inventoried.
        if kind == Kind::Other {
            other += 1;
            continue;
        }
        if f.size == 0 {
            zero.push(f.rel.clone());
        }
        let m = meta::read(&f.abs, &tools);
        let corrected = rename::corrected(&m, &offsets).map(fmt_dt);
        items.push(Item {
            path: f.rel.clone(),
            kind,
            camera: m.camera(),
            bytes: f.size,
            captured: m.captured.clone(),
            corrected,
            meta: m,
        });
    }
    if !zero.is_empty() {
        warnings.push(format!(
            "{} empty (0-byte) media files: {}",
            zero.len(),
            zero.join(", ")
        ));
    }
    let mut cams: BTreeMap<String, CameraSummary> = BTreeMap::new();
    let mut times: HashMap<String, Vec<NaiveDateTime>> = HashMap::new();
    for it in &items {
        let c = cams
            .entry(it.camera.clone())
            .or_insert_with(|| CameraSummary {
                camera: it.camera.clone(),
                clock_offset_minutes: rename::offset_for(&offsets, &it.camera),
                ..Default::default()
            });
        match it.kind {
            Kind::Video => c.videos += 1,
            Kind::Photo => c.photos += 1,
            Kind::Audio => c.audio += 1,
            Kind::Other => {}
        }
        c.bytes += it.bytes;
        c.video_seconds += it.meta.duration_s.unwrap_or(0.0);
        if let Some(f) = it.meta.capture_fps.clone().or(it.meta.format_fps.clone()) {
            c.frame_rates.insert(f);
        }
        if let (Some(w), Some(h)) = (it.meta.width, it.meta.height) {
            c.resolutions.insert(format!("{w}x{h}"));
        }
        if let Some(codec) = &it.meta.codec {
            c.codecs.insert(codec.clone());
        }
        if it
            .meta
            .source
            .as_deref()
            .is_some_and(|s| s.ends_with("file-time"))
        {
            c.times_from_file += 1;
        }
        if let Some(g) = &it.meta.gamma {
            c.gamma.insert(g.clone());
        }
        if let Some(l) = &it.meta.lens {
            c.lenses.insert(l.clone());
        }
        if let Some(t) = it
            .corrected
            .as_deref()
            .and_then(|s| NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S").ok())
        {
            times.entry(it.camera.clone()).or_default().push(t);
        }
    }
    for (cam, ts) in &mut times {
        ts.sort();
        if let Some(c) = cams.get_mut(cam) {
            c.first = ts.first().copied().map(fmt_dt);
            c.last = ts.last().copied().map(fmt_dt);
        }
    }
    for c in cams.values_mut() {
        c.video_seconds = (c.video_seconds * 10.0).round() / 10.0;
    }
    warnings.extend(clock_offset_warnings(&times));
    let csv = match csv_out {
        Some(p) => {
            write_csv(p, &items)?;
            Some(p.display().to_string())
        }
        None => None,
    };
    Ok(Inventory {
        folder: dir.display().to_string(),
        media_files: items.len(),
        other_files: other,
        card_files: card,
        bytes: items.iter().map(|i| i.bytes).sum(),
        cameras: cams.into_values().collect(),
        warnings,
        items,
        csv,
    })
}

/// Two cameras on one shoot whose recordings span about the same length of
/// time but sit a near-whole number of hours apart, without overlapping,
/// almost always means one camera's clock was set wrong (time zone, AM/PM).
pub fn clock_offset_warnings(times: &HashMap<String, Vec<NaiveDateTime>>) -> Vec<String> {
    let mut out = Vec::new();
    let mut cams: Vec<(&String, &Vec<NaiveDateTime>)> =
        times.iter().filter(|(_, t)| t.len() >= 3).collect();
    cams.sort_by_key(|(c, _)| (*c).clone());
    for i in 0..cams.len() {
        for j in i + 1..cams.len() {
            let (a, ta) = cams[i];
            let (b, tb) = cams[j];
            let (a0, a1) = (ta[0], *ta.last().unwrap());
            let (b0, b1) = (tb[0], *tb.last().unwrap());
            if a0 <= b1 && b0 <= a1 {
                continue; // overlapping: the clocks agree well enough
            }
            let span_a = (a1 - a0).num_minutes().max(1) as f64;
            let span_b = (b1 - b0).num_minutes().max(1) as f64;
            let ratio = span_a.max(span_b) / span_a.min(span_b);
            let mid_a = a0 + (a1 - a0) / 2;
            let mid_b = b0 + (b1 - b0) / 2;
            let diff_min = (mid_b - mid_a).num_minutes() as f64;
            let hours = (diff_min / 60.0).round();
            if hours.abs() >= 1.0 && (diff_min - hours * 60.0).abs() <= 25.0 && ratio <= 1.6 {
                let h = hours as i64;
                out.push(format!(
                    "{b} runs about {}{}h {} {a} with no overlap, though both cover a similar stretch of time. \
                     If this was one shoot, one camera's clock was off: re-run with --clock-offset \"{b}={}{}h\" \
                     (or \"{a}={}{}h\") to line them up",
                    if h > 0 { "" } else { "-" },
                    h.abs(),
                    if h > 0 { "later than" } else { "earlier than" },
                    if h > 0 { "-" } else { "+" },
                    h.abs(),
                    if h > 0 { "+" } else { "-" },
                    h.abs()
                ));
            }
        }
    }
    out
}

fn write_csv(path: &Path, items: &[Item]) -> Result<()> {
    if let Some(p) = path.parent() {
        fs::create_dir_all(p)?;
    }
    let mut w = csv::Writer::from_path(path)
        .with_context(|| format!("could not write {}", path.display()))?;
    w.write_record([
        "path",
        "kind",
        "camera",
        "captured",
        "corrected",
        "duration_s",
        "frame_rate",
        "resolution",
        "codec",
        "gamma",
        "gamut",
        "start_timecode",
        "lens",
        "iso",
        "bytes",
        "source",
    ])?;
    for it in items {
        let m = &it.meta;
        w.write_record([
            it.path.clone(),
            format!("{:?}", it.kind).to_ascii_lowercase(),
            it.camera.clone(),
            it.captured.clone().unwrap_or_default(),
            it.corrected.clone().unwrap_or_default(),
            m.duration_s.map(|d| d.to_string()).unwrap_or_default(),
            m.capture_fps
                .clone()
                .or(m.format_fps.clone())
                .unwrap_or_default(),
            match (m.width, m.height) {
                (Some(w), Some(h)) => format!("{w}x{h}"),
                _ => String::new(),
            },
            m.codec.clone().unwrap_or_default(),
            m.gamma.clone().unwrap_or_default(),
            m.gamut.clone().unwrap_or_default(),
            m.start_timecode.clone().unwrap_or_default(),
            m.lens.clone().unwrap_or_default(),
            m.iso.map(|i| i.to_string()).unwrap_or_default(),
            it.bytes.to_string(),
            m.source.clone().unwrap_or_default(),
        ])?;
    }
    w.flush()?;
    Ok(())
}

// ------------------------------------------------------------- report --

#[derive(Serialize, Debug)]
pub struct Report {
    pub folder: String,
    pub made: String,
    pub tool: String,
    pub inventory: Inventory,
    pub verification: Option<verify::Outcome>,
    pub manifests: Vec<String>,
    pub skipped_os_files: Vec<String>,
    pub warnings: Vec<String>,
    pub markdown: String,
    pub json: String,
}

pub struct ReportOptions {
    pub dir: PathBuf,
    /// What an offload already proved, for its own report.
    pub offload_note: Option<String>,
    pub manifest: Option<PathBuf>,
    pub verify: bool,
    pub only: Vec<String>,
    pub offsets: Vec<String>,
    pub out: Option<PathBuf>,
}

pub fn report(opts: &ReportOptions, stop: Arc<AtomicBool>) -> Result<Report> {
    let mut inv = inventory(&opts.dir, &opts.offsets, None)?;
    let walk = files::walk(&opts.dir)?;
    let mut manifests = Vec::new();
    let mhl = opts.dir.join(files::MHL_DIR);
    if let Ok(rd) = fs::read_dir(&mhl) {
        let n = rd
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().ends_with(".mhl"))
            .count();
        manifests.push(format!("{} ({n} ASC MHL generations)", mhl.display()));
    }
    if let Ok(rd) = fs::read_dir(opts.dir.join(files::DIT_DIR)) {
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().to_string();
            if n.ends_with(".tsv") {
                manifests.push(e.path().display().to_string());
            }
        }
    }
    if let Some(m) = &opts.manifest {
        manifests.push(m.display().to_string());
    }
    let verification = if opts.verify {
        Some(verify::run(
            &verify::Options {
                dir: opts.dir.clone(),
                manifest: opts.manifest.clone(),
                only: opts.only.clone(),
                record: false,
            },
            stop,
        )?)
    } else {
        None
    };
    let mut warnings = std::mem::take(&mut inv.warnings);
    if let Some(v) = &verification {
        warnings.extend(v.warnings.iter().cloned());
    }
    let made = chrono::Local::now()
        .format("%Y-%m-%d %H:%M:%S %:z")
        .to_string();
    let wanted_md = opts.out.clone().unwrap_or_else(|| {
        opts.dir.join(files::DIT_DIR).join(format!(
            "DIT-REPORT-{}.md",
            chrono::Local::now().format("%Y%m%d-%H%M%S")
        ))
    });
    if let Some(p) = wanted_md.parent() {
        fs::create_dir_all(p)?;
    }
    // A report made in the same second as another (an offload's own report,
    // then `report --verify`) gets the next free name: a DIT record is never
    // overwritten.
    let out_md = files::reserve_unique(&wanted_md, &["json"])?;
    let out_json = out_md.with_extension("json");

    let mut md = String::new();
    md.push_str(&format!(
        "# DIT report: {}\n\n",
        opts.dir
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default()
    ));
    md.push_str(&format!(
        "- Folder: `{}`\n- Made: {made} by nolgia dit {}\n",
        opts.dir.display(),
        env!("CARGO_PKG_VERSION")
    ));
    md.push_str(&format!(
        "- Media: {} files, {} ({} card-structure files and {} other files alongside)\n\n",
        inv.media_files,
        human_bytes(inv.bytes),
        inv.card_files,
        inv.other_files
    ));
    md.push_str("## Cameras\n\n| Camera | Videos | Photos | Size | First | Last | Clock offset | Video length | Frame rates | Resolution | Codec | Gamma | Lenses |\n|---|---|---|---|---|---|---|---|---|---|---|---|---|\n");
    for c in &inv.cameras {
        // Lens names like "24-70mm F2.8 DG DN | Art" carry a pipe, which would split a Markdown cell.
        let cell = |s: String| s.replace('|', "\\|");
        let join = |s: &BTreeSet<String>| cell(s.iter().cloned().collect::<Vec<_>>().join(", "));
        md.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} |\n",
            cell(c.camera.clone()),
            c.videos,
            c.photos,
            human_bytes(c.bytes),
            c.first.clone().unwrap_or_default(),
            c.last.clone().unwrap_or_default(),
            if c.clock_offset_minutes == 0 {
                "none".to_string()
            } else {
                format!("{:+} min", c.clock_offset_minutes)
            },
            if c.video_seconds > 0.0 {
                format!("{:.1} min", c.video_seconds / 60.0)
            } else {
                String::new()
            },
            join(&c.frame_rates),
            join(&c.resolutions),
            join(&c.codecs),
            join(&c.gamma),
            join(&c.lenses)
        ));
    }
    for c in inv.cameras.iter().filter(|c| c.times_from_file > 0) {
        md.push_str(&format!(
            "\n{}: {} of {} times are the files' modified times (the files record no capture time we can read).\n",
            c.camera.replace('|', "\\|"),
            c.times_from_file,
            c.videos + c.photos + c.audio
        ));
    }
    md.push_str("\n## Verification\n\n");
    match &verification {
        Some(v) => {
            md.push_str(&format!(
                "- Manifest: `{}` ({})\n- Checked {} of {} files ({}): **{} match**, {} differ, {} missing\n- Files on disk not in the manifest: {}\n- Took {}\n",
                v.manifest,
                v.hash,
                v.files_checked,
                v.files_in_manifest,
                human_bytes(v.bytes_checked),
                v.files_ok,
                v.mismatched.len(),
                v.missing.len(),
                v.not_in_manifest.len(),
                seconds_text(v.seconds)
            ));
            for p in v.mismatched.iter().chain(&v.missing) {
                md.push_str(&format!("  - `{}`: {}\n", p.path, p.detail));
            }
        }
        None => match &opts.offload_note {
            Some(note) => md.push_str(&format!("{note}\n")),
            None => md.push_str("Not run for this report (add --verify).\n"),
        },
    }
    md.push_str("\n## Manifests\n\n");
    if manifests.is_empty() {
        md.push_str("None found.\n");
    }
    for m in &manifests {
        md.push_str(&format!("- `{m}`\n"));
    }
    md.push_str("\n## Warnings\n\n");
    if warnings.is_empty() && walk.skipped.is_empty() {
        md.push_str("None.\n");
    }
    for w in &warnings {
        md.push_str(&format!("- {w}\n"));
    }
    if !walk.skipped.is_empty() {
        md.push_str(&format!(
            "- Operating-system files left out: {}\n",
            walk.skipped.join(", ")
        ));
    }
    let mut report = Report {
        folder: opts.dir.display().to_string(),
        made,
        tool: format!("nolgia dit {}", env!("CARGO_PKG_VERSION")),
        inventory: inv,
        verification,
        manifests,
        skipped_os_files: walk.skipped,
        warnings,
        markdown: out_md.display().to_string(),
        json: out_json.display().to_string(),
    };
    fs::write(&out_md, md).with_context(|| format!("could not write {}", out_md.display()))?;
    // The JSON copy carries every item; the returned value stays readable.
    fs::write(&out_json, serde_json::to_vec_pretty(&report)?)?;
    report.inventory.items.clear();
    Ok(report)
}
