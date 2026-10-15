//! `nolgia dit rename` and `nolgia dit undo`.
//!
//! Renames media in place from a pattern, keeping each file's original name
//! inside the new one so it can always be traced back. Sidecars (Sony
//! `M01.XML`, `.XMP`, `.THM`, `.LRV`, the JPG of a RAW+JPG pair) take their
//! clip's new name; card-structure files (thumbnails, databases, catalog
//! files) are never renamed.
//!
//! Safety:
//! - It is a dry run unless `--apply` is given.
//! - It refuses the whole rename if any two files would get the same name
//!   or a new name is already taken.
//! - With `--apply`, each line of `_DIT/rename-map.csv` is written and
//!   flushed to disk before its file is renamed, so a crash or a cancel can
//!   never leave a renamed file the map does not know about.
//! - `nolgia dit undo` (or the UNDO scripts written next to the map)
//!   restores every original name and never overwrites anything.

use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use anyhow::{Context, Result, bail};
use chrono::{Duration, NaiveDateTime};
use serde::Serialize;

use super::{
    files::{self, DIT_DIR},
    meta::{self, Kind, Tools},
};

/// Folders that belong to a camera card's own structure. Files under them
/// are thumbnails, indexes and catalogs, never renamed.
const CARD_DIRS: &[&str] = &[
    "thmbnl",
    "database",
    "canonmsc",
    "misc",
    "general",
    "sub",
    "take",
    "_card-files",
    "avf_info",
    "xdroot",
];
const CARD_FILES: &[&str] = &[
    "mediapro.xml",
    "cueup.xml",
    "status.bin",
    "index.bdm",
    "movieobj.bdm",
];

pub fn is_card_structure(rel: &str) -> bool {
    let lower = rel.to_ascii_lowercase();
    let mut parts: Vec<&str> = lower.split('/').collect();
    let name = parts.pop().unwrap_or("");
    CARD_FILES.contains(&name)
        || parts.iter().any(|p| CARD_DIRS.contains(p))
        || name.ends_with(".ctg")
}

/// `ILME-FX3A=-12h`, `Canon EOS R6m2=+1h30m`, `X=-90m`.
pub fn parse_offsets(list: &[String]) -> Result<HashMap<String, i64>> {
    let mut out = HashMap::new();
    for item in list {
        let Some((cam, off)) = item.rsplit_once('=') else {
            bail!("--clock-offset takes CAMERA=OFFSET, like \"ILME-FX3A=-12h\"; got {item:?}");
        };
        out.insert(cam.trim().to_string(), parse_minutes(off.trim())?);
    }
    Ok(out)
}

fn parse_minutes(s: &str) -> Result<i64> {
    let (sign, rest) = match s.strip_prefix('-') {
        Some(r) => (-1, r),
        None => (1, s.strip_prefix('+').unwrap_or(s)),
    };
    let mut total = 0i64;
    let mut num = String::new();
    for c in rest.chars() {
        if c.is_ascii_digit() {
            num.push(c);
        } else {
            let n: i64 = num.parse().with_context(|| format!("bad offset {s:?}"))?;
            num.clear();
            total += match c {
                'h' | 'H' => n * 60,
                'm' | 'M' => n,
                _ => bail!("bad offset {s:?}: use hours and minutes like -12h or +1h30m"),
            };
        }
    }
    if !num.is_empty() {
        bail!("bad offset {s:?}: end it with h or m, like -12h");
    }
    Ok(sign * total)
}

/// The clock offset for a camera, matched on its model name (case-insensitive).
pub fn offset_for(offsets: &HashMap<String, i64>, camera: &str) -> i64 {
    offsets
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(camera))
        .map(|(_, v)| *v)
        .unwrap_or(0)
}

pub fn corrected(meta: &meta::Meta, offsets: &HashMap<String, i64>) -> Option<NaiveDateTime> {
    meta.captured_dt()
        .map(|t| t + Duration::minutes(offset_for(offsets, &meta.camera())))
}

#[derive(Serialize, Debug, Clone)]
pub struct Change {
    pub from: String,
    pub to: String,
}

#[derive(Serialize, Debug)]
pub struct Outcome {
    /// `dry-run`, `applied`, `nothing-to-do` or `interrupted`.
    pub status: String,
    pub folder: String,
    pub pattern: String,
    pub cameras: BTreeMap<String, String>,
    pub to_rename: usize,
    pub already_named: usize,
    pub kept_card_files: usize,
    pub changes: Vec<Change>,
    pub map: Option<String>,
    pub undo: Option<String>,
    pub warnings: Vec<String>,
}

pub struct Options {
    pub dir: PathBuf,
    pub pattern: String,
    pub shoot: Option<String>,
    pub cameras: Vec<String>,
    pub offsets: Vec<String>,
    pub map: Option<PathBuf>,
    pub apply: bool,
}

/// Originals already recorded by an earlier rename (this command's map, or a
/// hand-made one with the same columns): new basename -> original stem.
fn known_originals(map: &Path) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let Ok(mut rdr) = csv::ReaderBuilder::new().flexible(true).from_path(map) else {
        return out;
    };
    for row in rdr.records().flatten() {
        if row.get(0) != Some("file") {
            continue;
        }
        let (Some(orig), Some(new)) = (row.get(1), row.get(2)) else {
            continue;
        };
        let base = |p: &str| p.rsplit(['/', '\\']).next().unwrap_or(p).to_string();
        let orig_stem = Path::new(&base(orig))
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        out.insert(base(new).to_ascii_lowercase(), orig_stem);
    }
    out
}

fn last_word(model: &str) -> String {
    model.split_whitespace().last().unwrap_or(model).to_string()
}

pub fn plan(opts: &Options) -> Result<(Outcome, Vec<(PathBuf, PathBuf)>)> {
    if !opts.dir.is_dir() {
        bail!("{} is not a folder", opts.dir.display());
    }
    if opts.pattern.contains("{shoot}") && opts.shoot.as_deref().is_none_or(|s| s.trim().is_empty())
    {
        bail!("the pattern uses {{shoot}}: give the shoot's name with --shoot");
    }
    if !opts.pattern.contains("{orig}") {
        bail!(
            "the pattern must keep {{orig}}, the camera's own name, so every file can be traced back"
        );
    }
    let offsets = parse_offsets(&opts.offsets)?;
    let mut labels: BTreeMap<String, String> = BTreeMap::new();
    for c in &opts.cameras {
        let Some((model, label)) = c.rsplit_once('=') else {
            bail!("--camera takes MODEL=LABEL, like \"ILME-FX3A=A-FX3A\"; got {c:?}");
        };
        labels.insert(
            model.trim().to_string(),
            files::sanitize_component(label.trim()),
        );
    }
    let map_path = opts
        .map
        .clone()
        .unwrap_or_else(|| opts.dir.join(DIT_DIR).join("rename-map.csv"));
    let originals = known_originals(&map_path);
    let tools = Tools::detect();
    let walk = files::walk(&opts.dir)?;
    let mut warnings = Vec::new();

    // Group files by folder and stem; pick each group's primary.
    let mut kept = 0usize;
    let mut groups: BTreeMap<(String, String), Vec<&files::Entry>> = BTreeMap::new();
    for f in &walk.files {
        if is_card_structure(&f.rel) {
            kept += 1;
            continue;
        }
        let (dir, name) = f.rel.rsplit_once('/').unwrap_or(("", &f.rel));
        let stem_full = Path::new(name)
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        // Sony's `C0001M01.XML` belongs to `C0001.MP4`.
        let ext = Path::new(name)
            .extension()
            .map(|e| e.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        let stem = if ext == "xml"
            && stem_full.len() > 3
            && stem_full.to_ascii_uppercase().ends_with("M01")
        {
            stem_full[..stem_full.len() - 3].to_string()
        } else {
            stem_full
        };
        groups
            .entry((dir.to_string(), stem.to_ascii_lowercase()))
            .or_default()
            .push(f);
    }
    let rank = |e: &files::Entry| -> u8 {
        let ext = e
            .abs
            .extension()
            .map(|x| x.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        match meta::kind_of(&e.abs) {
            Kind::Video => 0,
            Kind::Photo if !matches!(ext.as_str(), "jpg" | "jpeg" | "heic" | "heif" | "png") => 1,
            Kind::Photo => 2,
            Kind::Audio => 3,
            Kind::Other => 9,
        }
    };
    let mut camera_order: Vec<String> = Vec::new();
    let mut moves: Vec<(PathBuf, PathBuf)> = Vec::new();
    let mut changes = Vec::new();
    let mut already = 0usize;
    for ((_, _), mut members) in groups {
        members.sort_by_key(|e| (rank(e), e.rel.clone()));
        let primary = members[0];
        if rank(primary) == 9 {
            continue; // no media in this group
        }
        let m = meta::read(&primary.abs, &tools);
        let camera = m.camera();
        // Letters follow the order cameras first appear, labelled or not, so
        // an explicitly labelled A camera keeps the next one at B.
        if !camera_order.contains(&camera) {
            camera_order.push(camera.clone());
        }
        if !labels.contains_key(&camera) {
            let letter = (b'A'
                + (camera_order.iter().position(|c| c == &camera).unwrap_or(0) as u8).min(25))
                as char;
            labels.insert(
                camera.clone(),
                format!(
                    "{letter}-{}",
                    files::sanitize_component(&last_word(&camera))
                ),
            );
        }
        let Some(when) = corrected(&m, &offsets) else {
            warnings.push(format!("{}: no capture time, left as it is", primary.rel));
            continue;
        };
        let primary_name = primary
            .abs
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let orig = originals
            .get(&primary_name.to_ascii_lowercase())
            .cloned()
            .unwrap_or_else(|| {
                primary
                    .abs
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_default()
            });
        let new_stem = opts
            .pattern
            .replace(
                "{shoot}",
                &files::sanitize_component(opts.shoot.as_deref().unwrap_or("")),
            )
            .replace("{date}", &when.format("%Y%m%d").to_string())
            .replace("{time}", &when.format("%H%M").to_string())
            .replace("{camera}", &labels[&camera])
            .replace("{orig}", &orig);
        for member in members {
            let name = member
                .abs
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            let ext = member
                .abs
                .extension()
                .map(|e| e.to_string_lossy().to_string())
                .unwrap_or_default();
            let new_name = if ext.is_empty() {
                new_stem.clone()
            } else {
                format!("{new_stem}.{ext}")
            };
            if new_name == name {
                already += 1;
                continue;
            }
            let to = member.abs.with_file_name(&new_name);
            let to_rel = match member.rel.rsplit_once('/') {
                Some((d, _)) => format!("{d}/{new_name}"),
                None => new_name.clone(),
            };
            changes.push(Change {
                from: member.rel.clone(),
                to: to_rel,
            });
            moves.push((member.abs.clone(), to));
        }
    }
    // Refuse collisions: two files to one name, or a name already taken.
    let mut targets = HashSet::new();
    let sources: HashSet<String> = moves
        .iter()
        .map(|(f, _)| f.to_string_lossy().to_ascii_lowercase())
        .collect();
    for (_, to) in &moves {
        let key = to.to_string_lossy().to_ascii_lowercase();
        if !targets.insert(key.clone()) {
            bail!(
                "two files would both be named {}; change the pattern (add {{seq}} or {{time}})",
                to.display()
            );
        }
        if to.exists() && !sources.contains(&key) {
            bail!("{} already exists; nothing was renamed", to.display());
        }
    }
    if !moves.is_empty() {
        let stale = mhl_folders(&opts.dir);
        if !stale.is_empty() {
            warnings.push(format!(
                "ASC MHL manifests here list the current names ({}); after renaming, run `nolgia dit manifest` on each card folder to record the new names",
                stale.join(", ")
            ));
        }
    }
    let status = if moves.is_empty() {
        "nothing-to-do"
    } else {
        "dry-run"
    };
    let outcome = Outcome {
        status: status.into(),
        folder: opts.dir.display().to_string(),
        pattern: opts.pattern.clone(),
        cameras: labels,
        to_rename: moves.len(),
        already_named: already,
        kept_card_files: kept,
        changes,
        map: None,
        undo: None,
        warnings,
    };
    Ok((outcome, moves))
}

fn csv_line(fields: &[&str]) -> String {
    let q = |s: &str| {
        if s.contains([',', '"', '\n']) {
            format!("\"{}\"", s.replace('"', "\"\""))
        } else {
            s.to_string()
        }
    };
    let mut l = fields.iter().map(|f| q(f)).collect::<Vec<_>>().join(",");
    l.push('\n');
    l
}

pub fn apply(
    opts: &Options,
    mut outcome: Outcome,
    moves: Vec<(PathBuf, PathBuf)>,
    stop: Arc<AtomicBool>,
) -> Result<Outcome> {
    if moves.is_empty() {
        return Ok(outcome);
    }
    let dit = opts.dir.join(DIT_DIR);
    fs::create_dir_all(&dit)?;
    let map_path = opts
        .map
        .clone()
        .unwrap_or_else(|| dit.join("rename-map.csv"));
    let new_file = !map_path.exists();
    let mut map = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&map_path)
        .with_context(|| format!("could not open {}", map_path.display()))?;
    if new_file {
        map.write_all(csv_line(&["kind", "original_path", "new_path", "bytes"]).as_bytes())?;
        map.sync_data()?;
    }
    let rel = |p: &Path| files::rel_path(&opts.dir, p);
    let mut done = Vec::new();
    for (from, to) in &moves {
        if stop.load(Ordering::Relaxed) {
            outcome.status = "interrupted".into();
            break;
        }
        if to.exists() {
            bail!(
                "{} appeared during the rename; stopped, nothing was overwritten",
                to.display()
            );
        }
        let bytes = fs::metadata(from).map(|m| m.len()).unwrap_or(0).to_string();
        // The map line goes to disk first, then the rename.
        map.write_all(csv_line(&["file", &rel(from), &rel(to), &bytes]).as_bytes())?;
        map.sync_data()?;
        fs::rename(from, to).with_context(|| format!("could not rename {}", from.display()))?;
        done.push((from.clone(), to.clone()));
    }
    write_undo_scripts(&dit, &opts.dir, &done)?;
    if outcome.status != "interrupted" {
        outcome.status = "applied".into();
    }
    outcome.map = Some(map_path.display().to_string());
    outcome.undo = Some(format!("nolgia dit undo \"{}\"", opts.dir.display()));
    Ok(outcome)
}

/// Plain scripts for people without the CLI: PowerShell for Windows and a
/// shell script for macOS. Each line moves one file back and refuses to
/// overwrite.
fn write_undo_scripts(dit: &Path, root: &Path, done: &[(PathBuf, PathBuf)]) -> Result<()> {
    if done.is_empty() {
        return Ok(());
    }
    let ps_q = |s: &str| format!("'{}'", s.replace('\'', "''"));
    let sh_q = |s: &str| format!("'{}'", s.replace('\'', "'\\''"));
    let mut ps = String::from(
        "# Restores the original names from this rename. Never overwrites.\n$root = Split-Path -Parent $PSScriptRoot\n",
    );
    let mut sh = String::from(
        "#!/bin/sh\n# Restores the original names from this rename. Never overwrites.\ncd \"$(dirname \"$0\")/..\" || exit 1\n",
    );
    for (from, to) in done.iter().rev() {
        let (f, t) = (files::rel_path(root, from), files::rel_path(root, to));
        ps.push_str(&format!(
            "if (-not (Test-Path -LiteralPath (Join-Path $root {o}))) {{ Move-Item -LiteralPath (Join-Path $root {n}) -Destination (Join-Path $root {o}) }}\n",
            o = ps_q(&f.replace('/', "\\")),
            n = ps_q(&t.replace('/', "\\"))
        ));
        sh.push_str(&format!(
            "[ -e {o} ] || mv -n -- {n} {o}\n",
            o = sh_q(&f),
            n = sh_q(&t)
        ));
    }
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    fs::write(dit.join(format!("UNDO-RENAME-{stamp}.ps1")), ps)?;
    let shp = dit.join(format!("UNDO-RENAME-{stamp}.command"));
    fs::write(&shp, sh)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&shp, fs::Permissions::from_mode(0o755));
    }
    Ok(())
}

#[derive(Serialize, Debug)]
pub struct UndoOutcome {
    pub status: String,
    pub folder: String,
    pub map: String,
    pub restored: usize,
    pub skipped: Vec<Change>,
    pub warnings: Vec<String>,
}

/// The card folders in `dir` (or `dir` itself, as ".") that keep an ASC MHL
/// history, which a rename leaves listing the old names.
fn mhl_folders(dir: &Path) -> Vec<String> {
    let mut out: Vec<String> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().join(files::MHL_DIR).is_dir())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();
    if dir.join(files::MHL_DIR).is_dir() {
        out.push(".".to_string());
    }
    out.sort();
    out
}

/// Restore every original name in the map, newest first. A file whose
/// original name is taken again, or which is gone, is skipped and listed;
/// nothing is ever overwritten. A finished undo retires the map.
pub fn undo(dir: &Path, map: Option<&Path>, stop: Arc<AtomicBool>) -> Result<UndoOutcome> {
    let map_path = map
        .map(Path::to_path_buf)
        .unwrap_or_else(|| dir.join(DIT_DIR).join("rename-map.csv"));
    let mut rdr = csv::ReaderBuilder::new()
        .flexible(true)
        .from_path(&map_path)
        .with_context(|| format!("no rename map at {}", map_path.display()))?;
    let rows: Vec<csv::StringRecord> = rdr.records().collect::<Result<_, _>>()?;
    // A hand-made map may hold paths relative to the drive root; find the
    // base that makes its first new path exist.
    let base = |rel: &str| -> PathBuf {
        let direct = files::join_rel(dir, rel);
        if direct.exists() {
            return dir.to_path_buf();
        }
        dir.parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| dir.to_path_buf())
    };
    let root = rows
        .iter()
        .find(|r| r.get(0) == Some("file"))
        .and_then(|r| r.get(2))
        .map(base)
        .unwrap_or_else(|| dir.to_path_buf());
    let mut out = UndoOutcome {
        status: "restored".into(),
        folder: dir.display().to_string(),
        map: map_path.display().to_string(),
        restored: 0,
        skipped: Vec::new(),
        warnings: Vec::new(),
    };
    for row in rows.iter().rev() {
        if stop.load(Ordering::Relaxed) {
            out.status = "interrupted".into();
            break;
        }
        let (Some(kind), Some(orig), Some(new)) = (row.get(0), row.get(1), row.get(2)) else {
            continue;
        };
        let (o, n) = (files::join_rel(&root, orig), files::join_rel(&root, new));
        match kind {
            "file" => {
                if o.exists() {
                    out.skipped.push(Change {
                        from: new.into(),
                        to: format!("{orig} (already there, not overwritten)"),
                    });
                } else if !n.exists() {
                    out.skipped.push(Change {
                        from: new.into(),
                        to: format!("{orig} (file is gone)"),
                    });
                } else {
                    if let Some(p) = o.parent() {
                        fs::create_dir_all(p)?;
                    }
                    fs::rename(&n, &o)
                        .with_context(|| format!("could not restore {}", o.display()))?;
                    out.restored += 1;
                }
            }
            "dir" => {
                fs::create_dir_all(&o)?;
            }
            _ => {}
        }
    }
    if out.restored > 0 {
        let folders = mhl_folders(dir);
        if !folders.is_empty() {
            out.warnings.push(format!(
                "if you ran `nolgia dit manifest` after the rename, run it again on each card folder ({}) to record the original names",
                folders.join(", ")
            ));
        }
    }
    if out.status == "restored" {
        let retired = map_path.with_file_name(format!(
            "rename-map.undone-{}.csv",
            chrono::Local::now().format("%Y%m%d-%H%M%S")
        ));
        let _ = fs::rename(&map_path, retired);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offsets_parse() {
        assert_eq!(parse_minutes("-12h").unwrap(), -720);
        assert_eq!(parse_minutes("+1h30m").unwrap(), 90);
        assert_eq!(parse_minutes("45m").unwrap(), 45);
        assert!(parse_minutes("12").is_err());
    }

    #[test]
    fn card_structure_is_recognized() {
        assert!(is_card_structure("PRIVATE/M4ROOT/THMBNL/C0001T01.JPG"));
        assert!(is_card_structure("PRIVATE/M4ROOT/MEDIAPRO.XML"));
        assert!(is_card_structure("DCIM/CANONMSC/M0001.CTG"));
        assert!(!is_card_structure("PRIVATE/M4ROOT/CLIP/C0001.MP4"));
    }
}
