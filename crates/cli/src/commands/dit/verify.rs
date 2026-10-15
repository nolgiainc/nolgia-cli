//! `nolgia dit verify`: re-read every file a manifest lists and compare.
//! Read-only unless `--record` asks for a new ASC MHL generation.

use std::{
    collections::{BTreeMap, HashSet},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};

use anyhow::{Result, bail};
use serde::Serialize;

use super::{
    files,
    hash::{Algo, hash_file},
    manifest::{self, Process, Record},
};

#[derive(Serialize, Debug)]
pub struct Problem {
    pub path: String,
    pub detail: String,
}

#[derive(Serialize, Debug)]
pub struct Outcome {
    /// `ok`, `failed` (a mismatch or a missing file) or `interrupted`.
    pub status: String,
    pub folder: String,
    pub manifest: String,
    pub hash: &'static str,
    pub files_in_manifest: usize,
    pub files_checked: usize,
    pub files_ok: usize,
    pub bytes_checked: u64,
    pub mismatched: Vec<Problem>,
    pub missing: Vec<Problem>,
    /// On disk but not in the manifest: listed, not a failure (exports,
    /// notes and captures are added to a shoot folder all the time).
    pub not_in_manifest: Vec<String>,
    pub warnings: Vec<String>,
    pub seconds: f64,
    /// With `--record`: the new ASC MHL generation (the first one when a
    /// shoot folder's cards each got their own).
    pub recorded_generation: Option<String>,
    pub recorded_generations: Vec<String>,
}

pub struct Options {
    pub dir: PathBuf,
    pub manifest: Option<PathBuf>,
    pub only: Vec<String>,
    pub record: bool,
}

pub struct Loaded {
    pub label: String,
    pub records: BTreeMap<String, Record>,
    pub warnings: Vec<String>,
    /// For a shoot folder: the card folders inside it whose manifests were
    /// read (their records are keyed `<card>/<path>`). Empty otherwise.
    pub cards: Vec<String>,
}

/// Find and load the manifest: an explicit file (`.tsv` or one `.mhl`
/// generation) or folder (`ascmhl/`), else `<dir>/ascmhl`, else
/// `<dir>/_DIT/manifest.tsv`, else a shoot folder's cards: each folder
/// inside it with an `ascmhl/` (or a `_DIT/<card>.manifest.tsv` beside
/// them, as `nolgia dit offload` writes).
pub fn load(dir: &Path, manifest: Option<&Path>) -> Result<Loaded> {
    let one =
        |label: String, (records, warnings): (BTreeMap<String, Record>, Vec<String>)| Loaded {
            label,
            records,
            warnings,
            cards: Vec::new(),
        };
    if let Some(m) = manifest {
        if m.is_dir() {
            let root = if m
                .file_name()
                .is_some_and(|n| n.eq_ignore_ascii_case(files::MHL_DIR))
            {
                m.parent().unwrap_or(m).to_path_buf()
            } else {
                m.to_path_buf()
            };
            let label = root.join(files::MHL_DIR).display().to_string();
            return Ok(one(label, manifest::read_mhl_history(&root)?));
        }
        let name = m.to_string_lossy().to_ascii_lowercase();
        if name.ends_with(".mhl") {
            let root = m.parent().and_then(|p| p.parent()).unwrap_or(dir);
            return Ok(one(
                m.display().to_string(),
                manifest::read_mhl_history(root)?,
            ));
        }
        return Ok(one(
            m.display().to_string(),
            (manifest::read_tsv(m)?, Vec::new()),
        ));
    }
    if dir.join(files::MHL_DIR).is_dir() {
        let label = dir.join(files::MHL_DIR).display().to_string();
        return Ok(one(label, manifest::read_mhl_history(dir)?));
    }
    let tsv = manifest::tsv_path_for(dir);
    if tsv.is_file() {
        return Ok(one(
            tsv.display().to_string(),
            (manifest::read_tsv(&tsv)?, Vec::new()),
        ));
    }
    if let Some(shoot) = load_cards(dir)? {
        return Ok(shoot);
    }
    bail!(
        "no manifest found for {}: pass --manifest with a .tsv, an .mhl or an ascmhl folder",
        dir.display()
    )
}

/// A shoot folder: the manifest of each card folder inside it.
fn load_cards(dir: &Path) -> Result<Option<Loaded>> {
    let mut subs: Vec<String> = std::fs::read_dir(dir)?
        .flatten()
        .filter(|e| e.path().is_dir())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| !files::is_dit_internal(n))
        .collect();
    subs.sort();
    let mut out = Loaded {
        label: String::new(),
        records: BTreeMap::new(),
        warnings: Vec::new(),
        cards: Vec::new(),
    };
    let mut without = Vec::new();
    for card in subs {
        let root = dir.join(&card);
        let tsv = dir
            .join(files::DIT_DIR)
            .join(format!("{card}.manifest.tsv"));
        let (records, warnings) = if root.join(files::MHL_DIR).is_dir() {
            manifest::read_mhl_history(&root)?
        } else if tsv.is_file() {
            (manifest::read_tsv(&tsv)?, Vec::new())
        } else {
            without.push(card);
            continue;
        };
        for (rel, r) in records {
            let key = format!("{card}/{rel}");
            out.records.insert(key.clone(), Record { rel: key, ..r });
        }
        out.warnings
            .extend(warnings.into_iter().map(|w| format!("{card}: {w}")));
        out.cards.push(card);
    }
    if out.cards.is_empty() {
        return Ok(None);
    }
    if !without.is_empty() {
        out.warnings.push(format!(
            "no manifest in {}: its files are listed as not in the manifest",
            without.join(", ")
        ));
    }
    out.label = if out.cards.len() == 1 {
        dir.join(&out.cards[0])
            .join(files::MHL_DIR)
            .display()
            .to_string()
    } else {
        format!(
            "the manifests of {} cards in {}: {}",
            out.cards.len(),
            dir.display(),
            out.cards.join(", ")
        )
    };
    Ok(Some(out))
}

fn under(rel: &str, only: &[String]) -> bool {
    only.is_empty()
        || only.iter().any(|o| {
            let o = o.replace('\\', "/").trim_matches('/').to_ascii_lowercase();
            let r = rel.to_ascii_lowercase();
            r == o || r.starts_with(&format!("{o}/"))
        })
}

pub fn run(opts: &Options, stop: Arc<AtomicBool>) -> Result<Outcome> {
    let started = Instant::now();
    if !opts.dir.is_dir() {
        bail!("{} is not a folder", opts.dir.display());
    }
    let Loaded {
        label,
        records,
        mut warnings,
        cards,
    } = load(&opts.dir, opts.manifest.as_deref())?;
    let records: Vec<Record> = records
        .into_values()
        .filter(|r| under(&r.rel, &opts.only))
        .collect();
    let walk = files::walk(&opts.dir)?;
    let listed: HashSet<String> = records.iter().map(|r| r.rel.to_ascii_lowercase()).collect();
    let manifest_file = opts
        .manifest
        .as_ref()
        .map(|m| m.to_string_lossy().to_ascii_lowercase());
    let not_in_manifest: Vec<String> = walk
        .files
        .iter()
        .filter(|f| under(&f.rel, &opts.only))
        .filter(|f| !listed.contains(&f.rel.to_ascii_lowercase()))
        .filter(|f| {
            manifest_file.as_deref() != Some(f.abs.to_string_lossy().to_ascii_lowercase().as_str())
        })
        .map(|f| f.rel.clone())
        .collect();
    let algo = if records.iter().all(|r| r.hash(Algo::Xxh64).is_some()) {
        Algo::Xxh64
    } else if records.iter().all(|r| r.hash(Algo::Md5).is_some()) {
        Algo::Md5
    } else {
        Algo::Sha1
    };
    let total_bytes: u64 = records.iter().map(|r| r.size).sum();
    let mut out = Outcome {
        status: "ok".into(),
        folder: opts.dir.display().to_string(),
        manifest: label,
        hash: algo.name(),
        files_in_manifest: records.len(),
        files_checked: 0,
        files_ok: 0,
        bytes_checked: 0,
        mismatched: Vec::new(),
        missing: Vec::new(),
        not_in_manifest,
        warnings: Vec::new(),
        seconds: 0.0,
        recorded_generation: None,
        recorded_generations: Vec::new(),
    };
    let mut verified_records = Vec::new();
    let mut last = Instant::now();
    for r in &records {
        if stop.load(Ordering::Relaxed) {
            out.status = "interrupted".into();
            break;
        }
        let path = files::join_rel(&opts.dir, &r.rel);
        let Some(expected) = r.hash(algo) else {
            out.missing.push(Problem {
                path: r.rel.clone(),
                detail: format!("the manifest has no {} for it", algo.name()),
            });
            continue;
        };
        match hash_file(&path, &[algo]) {
            Ok((size, h)) => {
                out.files_checked += 1;
                out.bytes_checked += size;
                if h[0].1 == expected && size == r.size {
                    out.files_ok += 1;
                    verified_records.push(Record {
                        mtime: std::fs::metadata(&path)
                            .ok()
                            .and_then(|m| m.modified().ok()),
                        ..r.clone()
                    });
                } else if size != r.size {
                    out.mismatched.push(Problem {
                        path: r.rel.clone(),
                        detail: format!("size is {size}, the manifest says {}", r.size),
                    });
                } else {
                    out.mismatched.push(Problem {
                        path: r.rel.clone(),
                        detail: format!(
                            "{} is {}, the manifest says {expected}",
                            algo.name(),
                            h[0].1
                        ),
                    });
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                out.missing.push(Problem {
                    path: r.rel.clone(),
                    detail: "not on disk".into(),
                });
            }
            Err(e) => out.mismatched.push(Problem {
                path: r.rel.clone(),
                detail: format!("could not read it: {e}"),
            }),
        }
        if last.elapsed().as_secs() >= 2 {
            last = Instant::now();
            eprintln!(
                "verify: {}/{} files, {} of {}",
                out.files_checked,
                records.len(),
                super::offload::human_bytes(out.bytes_checked),
                super::offload::human_bytes(total_bytes)
            );
        }
    }
    if out.status != "interrupted" && (!out.mismatched.is_empty() || !out.missing.is_empty()) {
        out.status = "failed".into();
    }
    if opts.record && out.status != "interrupted" {
        let problems: Vec<String> = out
            .mismatched
            .iter()
            .chain(&out.missing)
            .map(|p| p.path.clone())
            .collect();
        let failed = |prefix: &str| problems.iter().any(|p| p.starts_with(prefix));
        // A shoot folder's cards each keep their own history.
        let roots: Vec<(String, PathBuf)> = if cards.is_empty() {
            vec![(String::new(), opts.dir.clone())]
        } else {
            cards
                .iter()
                .map(|c| (format!("{c}/"), opts.dir.join(c)))
                .collect()
        };
        for (prefix, root) in roots {
            let strip = |rel: &str| rel.strip_prefix(prefix.as_str()).map(str::to_string);
            let recs: Vec<Record> = verified_records
                .iter()
                .filter_map(|r| strip(&r.rel).map(|rel| Record { rel, ..r.clone() }))
                .collect();
            let empty: Vec<String> = walk.empty_dirs.iter().filter_map(|d| strip(d)).collect();
            let path = manifest::write_mhl_generation(
                &root,
                &recs,
                &empty,
                Process::InPlace,
                if failed(&prefix) {
                    "failed"
                } else {
                    "verified"
                },
            )?;
            out.recorded_generations.push(path.display().to_string());
        }
        out.recorded_generation = out.recorded_generations.first().cloned();
    }
    out.warnings.append(&mut warnings);
    out.seconds = (started.elapsed().as_secs_f64() * 10.0).round() / 10.0;
    Ok(out)
}
