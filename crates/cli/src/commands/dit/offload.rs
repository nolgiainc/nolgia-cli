//! `nolgia dit offload`: copy camera cards to one to three destinations and
//! prove every copy.
//!
//! The rules, in the order they protect the footage:
//! 1. The source is opened read-only and never written to.
//! 2. Nothing at a destination is ever replaced. A file already there with
//!    the same content counts as done; one with different content stops the
//!    whole offload before anything else is written (checked up front by
//!    size, and again by hash before a same-size file is trusted).
//! 3. Each file is read once, hashed while it streams to every destination
//!    (one writer thread per destination, at most `IN_FLIGHT` chunks
//!    buffered each), written to `name.nolgia-part`, flushed to disk, given
//!    the source's modification time and only then renamed into place.
//! 4. Every destination copy is then read back from disk and hashed again;
//!    only a copy whose hash matches the source's counts as verified.
//! 5. Progress is journaled one line per finished file, so a cancelled or
//!    interrupted offload resumes where it stopped: run the same command
//!    again. A half-written `.nolgia-part` is never mistaken for a copy.
//! 6. Cards are read one file at a time each, and at most `--max-parallel`
//!    cards at once, so a single card reader or USB hub is never thrashed.

use std::{
    collections::{BTreeMap, HashSet},
    fs,
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use super::{
    files::{self, DIT_DIR, Entry, PART_SUFFIX},
    hash::{Algo, CHUNK, MultiHasher, hash_file},
    manifest::{self, Process, Record},
};

/// Chunks buffered per destination writer: 4 x 8 MB.
const IN_FLIGHT: usize = 4;

pub struct Options {
    pub sources: Vec<PathBuf>,
    pub cards: Vec<String>,
    pub dests: Vec<PathBuf>,
    pub shoot: Option<String>,
    pub algos: Vec<Algo>,
    pub dry_run: bool,
    pub max_parallel: usize,
}

#[derive(Serialize, Clone, Debug)]
pub struct CardPlan {
    pub source: String,
    pub card: String,
    pub files: usize,
    pub bytes: u64,
    pub empty_folders: usize,
    pub skipped_os_files: Vec<String>,
}

#[derive(Serialize, Clone, Debug)]
pub struct DestPlan {
    pub destination: String,
    pub folder: String,
    pub free_bytes: Option<u64>,
    pub already_there_same_size: usize,
}

#[derive(Serialize, Clone, Debug)]
pub struct Conflict {
    pub path: String,
    pub reason: String,
}

#[derive(Serialize, Debug)]
pub struct Plan {
    pub cards: Vec<CardPlan>,
    pub destinations: Vec<DestPlan>,
    pub files: usize,
    pub bytes: u64,
    pub hashes: Vec<&'static str>,
    pub conflicts: Vec<Conflict>,
    pub warnings: Vec<String>,
}

#[derive(Serialize, Debug, Default)]
pub struct CardResult {
    pub card: String,
    pub destination: String,
    pub folder: String,
    pub verified: usize,
    pub already_there: usize,
    /// Copies an earlier run of this offload made and verified (its journal
    /// lists them); this run does not read them again.
    pub resumed: usize,
    pub failed: Vec<Conflict>,
    pub mhl: Option<String>,
    pub tsv: Option<String>,
}

#[derive(Serialize, Debug)]
pub struct Outcome {
    /// `complete`, `failed` (a mismatch or conflict), or `interrupted`.
    pub status: String,
    pub plan: Plan,
    pub results: Vec<CardResult>,
    /// Copies proven to match their card: in this run, or by an earlier run
    /// of the same offload (`files_done_earlier` of them).
    pub files_verified: usize,
    pub files_done_earlier: usize,
    /// Bytes copied in this run; `mb_per_s` is their rate.
    pub bytes_copied: u64,
    pub seconds: f64,
    pub mb_per_s: f64,
    /// The DIT report in the first destination's shoot folder.
    pub report: Option<String>,
    /// The DIT report written in each destination's shoot folder.
    pub reports: Vec<String>,
    pub resume_hint: Option<String>,
}

pub struct Card {
    source: PathBuf,
    name: String,
    walk: files::Walk,
}

fn card_root(dest: &Path, shoot: Option<&str>, card: &str) -> PathBuf {
    let mut p = dest.to_path_buf();
    if let Some(s) = shoot {
        p.push(files::sanitize_component(s));
    }
    p.push(card);
    p
}

/// `_DIT` sits beside the cards: `<dest>/<shoot>/_DIT`, so the card folders
/// themselves hold nothing but media, the card structure and `ascmhl/`.
pub fn dit_dir(dest: &Path, shoot: Option<&str>) -> PathBuf {
    let mut p = dest.to_path_buf();
    if let Some(s) = shoot {
        p.push(files::sanitize_component(s));
    }
    p.push(DIT_DIR);
    p
}

fn canonical(p: &Path) -> PathBuf {
    fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
}

/// Closest existing ancestor, for the free-space check on a folder that
/// does not exist yet.
fn existing_ancestor(p: &Path) -> Option<PathBuf> {
    let mut cur = Some(p);
    while let Some(c) = cur {
        if c.exists() {
            return Some(c.to_path_buf());
        }
        cur = c.parent();
    }
    None
}

pub fn plan(opts: &Options) -> Result<(Plan, Vec<Card>)> {
    if opts.sources.is_empty() {
        bail!("name at least one card or folder to offload");
    }
    if opts.dests.is_empty() || opts.dests.len() > 3 {
        bail!("give one to three destinations with --to");
    }
    let mut warnings = Vec::new();
    let mut cards = Vec::new();
    let mut names = HashSet::new();
    for (i, src) in opts.sources.iter().enumerate() {
        if !src.is_dir() {
            bail!("{} is not a folder or mounted card", src.display());
        }
        let name = opts
            .cards
            .get(i)
            .map(|n| files::sanitize_component(n))
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| files::default_card_name(src, i));
        if !names.insert(name.to_ascii_lowercase()) {
            bail!(
                "two cards would both be named {name:?}; name them with --card, once per source in order"
            );
        }
        let walk = files::walk(src)?;
        // Case-only twins collide on Windows and macOS destinations.
        let mut seen = HashSet::new();
        for f in &walk.files {
            if !seen.insert(f.rel.to_ascii_lowercase()) {
                bail!(
                    "{} has two files whose names differ only by upper/lower case ({}); \
                     they would overwrite each other on a Windows or macOS drive",
                    src.display(),
                    f.rel
                );
            }
        }
        cards.push(Card {
            source: src.clone(),
            name,
            walk,
        });
    }
    let csrc: Vec<PathBuf> = opts.sources.iter().map(|s| canonical(s)).collect();
    for d in &opts.dests {
        let cd = canonical(&existing_ancestor(d).unwrap_or_else(|| d.clone()));
        for s in &csrc {
            if cd.starts_with(s) {
                bail!(
                    "the destination {} is inside the card {}; pick a folder on another drive",
                    d.display(),
                    s.display()
                );
            }
        }
    }
    let total_files: usize = cards.iter().map(|c| c.walk.files.len()).sum();
    let total_bytes: u64 = cards
        .iter()
        .flat_map(|c| &c.walk.files)
        .map(|f| f.size)
        .sum();
    let mut conflicts = Vec::new();
    let mut dest_plans = Vec::new();
    for d in &opts.dests {
        let mut same = 0usize;
        let mut needed = 0u64;
        for c in &cards {
            let root = card_root(d, opts.shoot.as_deref(), &c.name);
            for f in &c.walk.files {
                let target = files::join_rel(&root, &f.rel);
                match fs::metadata(&target) {
                    Ok(m) if m.is_file() && m.len() == f.size => same += 1,
                    Ok(m) if m.is_file() => conflicts.push(Conflict {
                        path: target.display().to_string(),
                        reason: format!(
                            "already exists with different content ({} bytes there, {} on the card); it was not touched",
                            m.len(),
                            f.size
                        ),
                    }),
                    Ok(_) => conflicts.push(Conflict {
                        path: target.display().to_string(),
                        reason: "a folder is in the way of this file; it was not touched".to_string(),
                    }),
                    Err(_) => needed += f.size,
                }
            }
        }
        let free = existing_ancestor(d).and_then(|a| fs2::available_space(a).ok());
        if let Some(free) = free
            && free < needed
        {
            bail!(
                "{} has {} free but this offload needs {}",
                d.display(),
                human_bytes(free),
                human_bytes(needed)
            );
        }
        dest_plans.push(DestPlan {
            destination: d.display().to_string(),
            folder: match &opts.shoot {
                Some(s) => d.join(files::sanitize_component(s)).display().to_string(),
                None => d.display().to_string(),
            },
            free_bytes: free,
            already_there_same_size: same,
        });
    }
    if cards.iter().any(|c| !c.walk.skipped.is_empty()) {
        warnings.push("operating-system files on the card (trash, indexes) are not copied; they are listed per card".to_string());
    }
    let plan = Plan {
        cards: cards
            .iter()
            .map(|c| CardPlan {
                source: c.source.display().to_string(),
                card: c.name.clone(),
                files: c.walk.files.len(),
                bytes: c.walk.files.iter().map(|f| f.size).sum(),
                empty_folders: c.walk.empty_dirs.len(),
                skipped_os_files: c.walk.skipped.clone(),
            })
            .collect(),
        destinations: dest_plans,
        files: total_files,
        bytes: total_bytes,
        hashes: opts.algos.iter().map(|a| a.name()).collect(),
        conflicts,
        warnings,
    };
    Ok((plan, cards))
}

// ------------------------------------------------------------ journal --

#[derive(Serialize, Deserialize)]
struct Done {
    rel: String,
    size: u64,
    hashes: Vec<(String, String)>,
}

fn journal_path(dest: &Path, shoot: Option<&str>, card: &str) -> PathBuf {
    dit_dir(dest, shoot)
        .join(".state")
        .join(format!("offload-{card}.jsonl"))
}

fn load_journal(path: &Path) -> BTreeMap<String, Done> {
    let mut out = BTreeMap::new();
    if let Ok(f) = fs::File::open(path) {
        for line in BufReader::new(f).lines().map_while(Result::ok) {
            if let Ok(d) = serde_json::from_str::<Done>(&line) {
                out.insert(d.rel.clone(), d);
            }
        }
    }
    out
}

struct Journal {
    file: fs::File,
}

impl Journal {
    fn open(path: &Path) -> Result<Self> {
        if let Some(p) = path.parent() {
            fs::create_dir_all(p)?;
        }
        let file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;
        Ok(Self { file })
    }
    fn record(&mut self, d: &Done) -> Result<()> {
        let mut line = serde_json::to_string(d)?;
        line.push('\n');
        self.file.write_all(line.as_bytes())?;
        self.file.sync_data()?;
        Ok(())
    }
}

// ------------------------------------------------------------ copying --

pub struct Progress {
    /// Bytes done, counting files an earlier run finished (for the progress line).
    pub bytes: AtomicU64,
    /// Bytes read from the cards and written in this run.
    pub copied: AtomicU64,
    pub files: AtomicU64,
    pub total_bytes: u64,
    pub total_files: u64,
    pub stop: Arc<AtomicBool>,
}

/// Test-only failpoint: stop as if interrupted after this many files.
fn fail_after() -> Option<u64> {
    std::env::var("NOLGIA_DIT_TEST_STOP_AFTER_FILES")
        .ok()
        .and_then(|v| v.parse().ok())
}

enum CopyEnd {
    Done(Vec<(Algo, String)>),
    Stopped,
}

/// Stream one source file to every target's `.nolgia-part`, hashing as it
/// goes; finish each part (flush, mtime, rename) only when the whole file
/// arrived. Returns the source hashes.
fn copy_file(
    src: &Entry,
    targets: &[PathBuf],
    algos: &[Algo],
    progress: &Progress,
) -> Result<CopyEnd> {
    let mut input = fs::File::open(&src.abs)
        .with_context(|| format!("could not read {}", src.abs.display()))?;
    let mut parts = Vec::new();
    for t in targets {
        if let Some(p) = t.parent() {
            fs::create_dir_all(p).with_context(|| format!("could not create {}", p.display()))?;
        }
        let part = PathBuf::from(format!("{}{PART_SUFFIX}", t.display()));
        // A part left by an interrupted run is ours and incomplete.
        let _ = fs::remove_file(&part);
        parts.push(part);
    }
    let mut hasher = MultiHasher::new(algos);
    let stopped = thread::scope(|scope| -> Result<bool> {
        let mut senders = Vec::new();
        let mut handles = Vec::new();
        for part in &parts {
            let (tx, rx) = mpsc::sync_channel::<Arc<Vec<u8>>>(IN_FLIGHT);
            let part = part.clone();
            handles.push(scope.spawn(move || -> Result<()> {
                let mut out = fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&part)
                    .with_context(|| format!("could not create {}", part.display()))?;
                for chunk in rx {
                    out.write_all(&chunk)
                        .with_context(|| format!("could not write {}", part.display()))?;
                }
                out.sync_all()
                    .with_context(|| format!("could not flush {}", part.display()))?;
                Ok(())
            }));
            senders.push(tx);
        }
        let mut stopped = false;
        loop {
            if progress.stop.load(Ordering::Relaxed) {
                stopped = true;
                break;
            }
            let mut buf = vec![0u8; CHUNK];
            let n = input
                .read(&mut buf)
                .with_context(|| format!("could not read {}", src.abs.display()))?;
            if n == 0 {
                break;
            }
            buf.truncate(n);
            hasher.update(&buf);
            let chunk = Arc::new(buf);
            for tx in &senders {
                if tx.send(chunk.clone()).is_err() {
                    break;
                }
            }
            progress.bytes.fetch_add(n as u64, Ordering::Relaxed);
        }
        drop(senders);
        for h in handles {
            h.join()
                .map_err(|_| anyhow::anyhow!("a destination writer stopped unexpectedly"))??;
        }
        Ok(stopped)
    })?;
    if stopped {
        for p in &parts {
            let _ = fs::remove_file(p);
        }
        return Ok(CopyEnd::Stopped);
    }
    for (part, target) in parts.iter().zip(targets) {
        if let Some(t) = src.mtime {
            let _ = filetime::set_file_mtime(part, filetime::FileTime::from_system_time(t));
        }
        // Never replace: a target that appeared since the plan stops here.
        if target.exists() {
            let _ = fs::remove_file(part);
            bail!(
                "{} appeared while copying; it was not replaced",
                target.display()
            );
        }
        fs::rename(part, target)
            .with_context(|| format!("could not finish {}", target.display()))?;
    }
    Ok(CopyEnd::Done(hasher.finish()))
}

pub fn run(
    opts: &Options,
    stop: Arc<AtomicBool>,
    plan_only: Plan,
    cards: Vec<Card>,
) -> Result<Outcome> {
    let started = Instant::now();
    let progress = Arc::new(Progress {
        bytes: AtomicU64::new(0),
        copied: AtomicU64::new(0),
        files: AtomicU64::new(0),
        total_bytes: plan_only.bytes,
        total_files: plan_only.files as u64,
        stop: stop.clone(),
    });
    let results: Mutex<Vec<CardResult>> = Mutex::new(Vec::new());
    let failed = AtomicBool::new(false);
    let interrupted = AtomicBool::new(false);
    let reporter = spawn_progress(progress.clone(), opts.dests.clone(), opts.shoot.clone());
    let stop_after = fail_after();
    let errors: Mutex<Vec<anyhow::Error>> = Mutex::new(Vec::new());

    let queue = Mutex::new(cards.into_iter().collect::<Vec<_>>());
    let workers = opts.max_parallel.clamp(1, 4);
    thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| {
                loop {
                    let card = match queue.lock().unwrap().pop() {
                        Some(c) => c,
                        None => break,
                    };
                    match offload_card(opts, &card, &progress, stop_after) {
                        Ok((res, card_failed, card_stopped)) => {
                            if card_failed {
                                failed.store(true, Ordering::Relaxed);
                            }
                            if card_stopped {
                                interrupted.store(true, Ordering::Relaxed);
                            }
                            results.lock().unwrap().extend(res);
                        }
                        Err(e) => {
                            failed.store(true, Ordering::Relaxed);
                            stop.store(true, Ordering::Relaxed);
                            errors.lock().unwrap().push(e);
                        }
                    }
                }
            });
        }
    });
    progress.stop.store(true, Ordering::Relaxed);
    let _ = reporter.join();
    if let Some(e) = errors.into_inner().unwrap().into_iter().next() {
        return Err(e);
    }
    let mut results = results.into_inner().unwrap();
    results.sort_by(|a, b| (&a.destination, &a.card).cmp(&(&b.destination, &b.card)));
    let seconds = started.elapsed().as_secs_f64();
    let bytes = progress.copied.load(Ordering::Relaxed);
    let status = if interrupted.load(Ordering::Relaxed) {
        "interrupted"
    } else if failed.load(Ordering::Relaxed) {
        "failed"
    } else {
        "complete"
    };
    Ok(Outcome {
        status: status.to_string(),
        files_verified: results
            .iter()
            .map(|r| r.verified + r.already_there + r.resumed)
            .sum(),
        files_done_earlier: results.iter().map(|r| r.resumed).sum(),
        plan: plan_only,
        results,
        bytes_copied: bytes,
        seconds: (seconds * 10.0).round() / 10.0,
        mb_per_s: if seconds > 0.0 {
            ((bytes as f64 / 1e6 / seconds) * 10.0).round() / 10.0
        } else {
            0.0
        },
        report: None,
        reports: Vec::new(),
        resume_hint: (status == "interrupted")
            .then(|| "run the same command again to resume".to_string()),
    })
}

fn offload_card(
    opts: &Options,
    card: &Card,
    progress: &Progress,
    stop_after: Option<u64>,
) -> Result<(Vec<CardResult>, bool, bool)> {
    let shoot = opts.shoot.as_deref();
    let roots: Vec<PathBuf> = opts
        .dests
        .iter()
        .map(|d| card_root(d, shoot, &card.name))
        .collect();
    let mut journals = Vec::new();
    let mut done_before = Vec::new();
    for d in &opts.dests {
        let jp = journal_path(d, shoot, &card.name);
        done_before.push(load_journal(&jp));
        journals.push(Journal::open(&jp)?);
    }
    let mut results: Vec<CardResult> = opts
        .dests
        .iter()
        .zip(&roots)
        .map(|(d, r)| CardResult {
            card: card.name.clone(),
            destination: d.display().to_string(),
            folder: r.display().to_string(),
            ..Default::default()
        })
        .collect();
    for root in &roots {
        fs::create_dir_all(root).with_context(|| format!("could not create {}", root.display()))?;
        for d in &card.walk.empty_dirs {
            fs::create_dir_all(files::join_rel(root, d))?;
        }
    }
    let mut records: Vec<Vec<Record>> = vec![Vec::new(); roots.len()];
    let mut any_failed = false;
    for f in &card.walk.files {
        if progress.stop.load(Ordering::Relaxed) {
            return Ok((results, any_failed, true));
        }
        let targets: Vec<PathBuf> = roots.iter().map(|r| files::join_rel(r, &f.rel)).collect();
        // Which destinations still need this file?
        let mut need = Vec::new();
        for (i, t) in targets.iter().enumerate() {
            let journaled = done_before[i].get(&f.rel).filter(|d| d.size == f.size);
            match (journaled, fs::metadata(t)) {
                (Some(d), Ok(m)) if m.len() == f.size => {
                    results[i].resumed += 1;
                    records[i].push(Record {
                        rel: f.rel.clone(),
                        size: f.size,
                        mtime: f.mtime,
                        hashes: d
                            .hashes
                            .iter()
                            .filter_map(|(a, h)| Algo::parse(a).map(|a| (a, h.clone())))
                            .collect(),
                    });
                }
                _ => need.push(i),
            }
        }
        if need.is_empty() {
            progress.files.fetch_add(1, Ordering::Relaxed);
            progress.bytes.fetch_add(f.size, Ordering::Relaxed);
            continue;
        }
        // A same-size file already at a destination: identical content is
        // done, different content stops everything. Never replaced.
        let mut copy_to = Vec::new();
        let mut source_hashes: Option<Vec<(Algo, String)>> = None;
        for &i in &need {
            if targets[i].exists() {
                let src_h = match &source_hashes {
                    Some(h) => h.clone(),
                    None => {
                        let (_, h) = hash_file(&f.abs, &opts.algos)
                            .with_context(|| format!("could not read {}", f.abs.display()))?;
                        source_hashes = Some(h.clone());
                        h
                    }
                };
                let (_, there) = hash_file(&targets[i], &[Algo::Xxh64])?;
                if there[0].1 == src_h[0].1 {
                    results[i].already_there += 1;
                    let rec = Record {
                        rel: f.rel.clone(),
                        size: f.size,
                        mtime: f.mtime,
                        hashes: src_h.clone(),
                    };
                    journals[i].record(&done_of(&rec))?;
                    records[i].push(rec);
                } else {
                    bail!(
                        "{} already exists with different content; nothing there was replaced. \
                         Move it aside or pick another destination, then run the offload again",
                        targets[i].display()
                    );
                }
            } else {
                copy_to.push(i);
            }
        }
        if !copy_to.is_empty() {
            let t: Vec<PathBuf> = copy_to.iter().map(|&i| targets[i].clone()).collect();
            let hashes = match copy_file(f, &t, &opts.algos, progress)? {
                CopyEnd::Done(h) => h,
                CopyEnd::Stopped => return Ok((results, any_failed, true)),
            };
            progress.copied.fetch_add(f.size, Ordering::Relaxed);
            // Read every copy back from disk and compare.
            let checks: Vec<(usize, Result<String>)> = thread::scope(|s| {
                let hs: Vec<_> = copy_to
                    .iter()
                    .map(|&i| {
                        let p = targets[i].clone();
                        s.spawn(move || {
                            (
                                i,
                                hash_file(&p, &[Algo::Xxh64])
                                    .map(|(_, h)| h[0].1.clone())
                                    .map_err(anyhow::Error::from),
                            )
                        })
                    })
                    .collect();
                hs.into_iter()
                    .map(|h| h.join().expect("verify thread"))
                    .collect()
            });
            for (i, check) in checks {
                match check {
                    Ok(h) if h == hashes[0].1 => {
                        results[i].verified += 1;
                        let rec = Record {
                            rel: f.rel.clone(),
                            size: f.size,
                            mtime: f.mtime,
                            hashes: hashes.clone(),
                        };
                        journals[i].record(&done_of(&rec))?;
                        records[i].push(rec);
                    }
                    Ok(h) => {
                        any_failed = true;
                        results[i].failed.push(Conflict {
                            path: targets[i].display().to_string(),
                            reason: format!(
                                "the copy reads back as {h}, the card as {}; the copy is bad",
                                hashes[0].1
                            ),
                        });
                    }
                    Err(e) => {
                        any_failed = true;
                        results[i].failed.push(Conflict {
                            path: targets[i].display().to_string(),
                            reason: format!("could not read the copy back: {e}"),
                        });
                    }
                }
            }
        } else {
            progress.bytes.fetch_add(f.size, Ordering::Relaxed);
        }
        let n = progress.files.fetch_add(1, Ordering::Relaxed) + 1;
        if stop_after.is_some_and(|k| n >= k) {
            progress.stop.store(true, Ordering::Relaxed);
        }
    }
    // Manifests describe exactly the verified copies. A copy this card's
    // ASC MHL history already lists (same size and xxh64, from an earlier run
    // of the offload) is not listed again: a second "original" would claim
    // a copy this run did not make.
    for (i, root) in roots.iter().enumerate() {
        if records[i].is_empty() {
            continue;
        }
        let known = if root.join(files::MHL_DIR).is_dir() {
            manifest::read_mhl_history(root)
                .map(|(known, _)| known)
                .unwrap_or_default()
        } else {
            BTreeMap::new()
        };
        let fresh: Vec<Record> = records[i]
            .iter()
            .filter(|r| {
                !known.get(&r.rel).is_some_and(|k| {
                    k.size == r.size
                        && k.hash(Algo::Xxh64).is_some()
                        && k.hash(Algo::Xxh64) == r.hash(Algo::Xxh64)
                })
            })
            .cloned()
            .collect();
        let mhl = if fresh.is_empty() {
            root.join(files::MHL_DIR)
        } else {
            manifest::write_mhl_generation(
                root,
                &fresh,
                &card.walk.empty_dirs,
                Process::Transfer,
                "original",
            )?
        };
        let tsv = dit_dir(&opts.dests[i], shoot).join(format!("{}.manifest.tsv", card.name));
        manifest::write_tsv(&tsv, &root.display().to_string(), &records[i], &opts.algos)?;
        results[i].mhl = Some(mhl.display().to_string());
        results[i].tsv = Some(tsv.display().to_string());
    }
    Ok((results, any_failed, false))
}

fn done_of(r: &Record) -> Done {
    Done {
        rel: r.rel.clone(),
        size: r.size,
        hashes: r
            .hashes
            .iter()
            .map(|(a, h)| (a.name().to_string(), h.clone()))
            .collect(),
    }
}

/// Every 2 s: a line on stderr for people, and `_DIT/offload-progress.json`
/// at each destination for the desktop plugins' status checks.
fn spawn_progress(
    p: Arc<Progress>,
    dests: Vec<PathBuf>,
    shoot: Option<String>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let start = Instant::now();
        let mut last = Instant::now();
        let mut finished = false;
        loop {
            let stop = p.stop.load(Ordering::Relaxed);
            if last.elapsed() >= Duration::from_secs(2) || stop {
                last = Instant::now();
                let bytes = p.bytes.load(Ordering::Relaxed);
                let files = p.files.load(Ordering::Relaxed);
                let secs = start.elapsed().as_secs_f64().max(0.001);
                let rate = bytes as f64 / 1e6 / secs;
                if !finished {
                    eprintln!(
                        "offload: {files}/{} files, {} of {}, {rate:.0} MB/s",
                        p.total_files,
                        human_bytes(bytes),
                        human_bytes(p.total_bytes)
                    );
                }
                let status = serde_json::json!({
                    "files_done": files, "files_total": p.total_files,
                    "bytes_done": bytes, "bytes_total": p.total_bytes,
                    "mb_per_s": (rate * 10.0).round() / 10.0,
                    "running": !stop,
                });
                for d in &dests {
                    let path = dit_dir(d, shoot.as_deref()).join("offload-progress.json");
                    if path.parent().is_some_and(|p| p.exists()) {
                        let _ = manifest::write_synced(&path, status.to_string().as_bytes());
                    }
                }
                if stop {
                    finished = true;
                }
            }
            if finished {
                break;
            }
            thread::sleep(Duration::from_millis(200));
        }
    })
}

/// "0.6 s" under ten seconds, "42 s" above.
pub fn seconds_text(s: f64) -> String {
    if s < 10.0 {
        format!("{s:.1} s")
    } else {
        format!("{s:.0} s")
    }
}

pub fn human_bytes(b: u64) -> String {
    let b = b as f64;
    if b >= 1e12 {
        format!("{:.2} TB", b / 1e12)
    } else if b >= 1e9 {
        format!("{:.2} GB", b / 1e9)
    } else if b >= 1e6 {
        format!("{:.1} MB", b / 1e6)
    } else if b >= 1e3 {
        format!("{:.1} KB", b / 1e3)
    } else {
        format!("{b} B")
    }
}
