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
    pub recorded_generation: Option<String>,
}

pub struct Options {
    pub dir: PathBuf,
    pub manifest: Option<PathBuf>,
    pub only: Vec<String>,
    pub record: bool,
}

/// Find and load the manifest: an explicit file (`.tsv` or one `.mhl`
/// generation) or folder (`ascmhl/`), else `<dir>/ascmhl`, else
/// `<dir>/_DIT/manifest.tsv`.
pub fn load(
    dir: &Path,
    manifest: Option<&Path>,
) -> Result<(String, BTreeMap<String, Record>, Vec<String>)> {
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
            let (recs, warn) = manifest::read_mhl_history(&root)?;
            return Ok((root.join(files::MHL_DIR).display().to_string(), recs, warn));
        }
        let name = m.to_string_lossy().to_ascii_lowercase();
        if name.ends_with(".mhl") {
            let root = m.parent().and_then(|p| p.parent()).unwrap_or(dir);
            let (recs, warn) = manifest::read_mhl_history(root)?;
            return Ok((m.display().to_string(), recs, warn));
        }
        return Ok((m.display().to_string(), manifest::read_tsv(m)?, Vec::new()));
    }
    if dir.join(files::MHL_DIR).is_dir() {
        let (recs, warn) = manifest::read_mhl_history(dir)?;
        return Ok((dir.join(files::MHL_DIR).display().to_string(), recs, warn));
    }
    let tsv = manifest::tsv_path_for(dir);
    if tsv.is_file() {
        return Ok((
            tsv.display().to_string(),
            manifest::read_tsv(&tsv)?,
            Vec::new(),
        ));
    }
    bail!(
        "no manifest found for {}: pass --manifest with a .tsv, an .mhl or an ascmhl folder",
        dir.display()
    )
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
    let (label, records, mut warnings) = load(&opts.dir, opts.manifest.as_deref())?;
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
        let path = manifest::write_mhl_generation(
            &opts.dir,
            &verified_records,
            &walk.empty_dirs,
            Process::InPlace,
            if out.status == "ok" {
                "verified"
            } else {
                "failed"
            },
        )?;
        out.recorded_generation = Some(path.display().to_string());
    }
    out.warnings.append(&mut warnings);
    out.seconds = (started.elapsed().as_secs_f64() * 10.0).round() / 10.0;
    Ok(out)
}
