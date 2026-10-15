//! Manifests: ASC MHL v2 (the industry format that Silverstack, ShotPut,
//! Hedge and the reference `ascmhl` tool read and verify) plus a plain TSV
//! that any spreadsheet opens.
//!
//! MHL layout, as the ASC spec and the reference tool write it: a folder
//! `ascmhl/` at the root of the media folder holding one generation file per
//! hashing pass (`0001_<root>_<YYYY-MM-DD_HHMMSSZ>.mhl`) and
//! `ascmhl_chain.xml`, which lists every generation with the C4 ID of its
//! bytes so a later edit to a generation file is detectable. Directory and
//! root hashes follow the reference tool's algorithm exactly (see
//! `dir_hashes`), so `ascmhl verify` accepts our output.

use std::{
    collections::{BTreeMap, HashMap},
    fs,
    io::Write,
    path::{Path, PathBuf},
    time::SystemTime,
};

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Local, Utc};
use quick_xml::{Reader, events::Event};
use sha2::{Digest, Sha512};

use super::files::{DIT_DIR, MHL_DIR, PART_SUFFIX};
use super::hash::Algo;

/// One file's line in a manifest.
#[derive(Clone, Debug, PartialEq)]
pub struct Record {
    pub rel: String,
    pub size: u64,
    pub mtime: Option<SystemTime>,
    /// Lowercase hex digests, xxh64 first when present.
    pub hashes: Vec<(Algo, String)>,
}

impl Record {
    pub fn hash(&self, algo: Algo) -> Option<&str> {
        self.hashes
            .iter()
            .find(|(a, _)| *a == algo)
            .map(|(_, h)| h.as_str())
    }
}

// ---------------------------------------------------------------- C4 ID --

const C4_ALPHABET: &[u8] = b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";

/// The C4 ID (SMPTE ST 2114) of some bytes: SHA-512, read as a 512-bit
/// big-endian number, written in base 58 padded to 88 digits, prefixed `c4`.
pub fn c4_id(data: &[u8]) -> String {
    let digest = Sha512::digest(data);
    let mut num: Vec<u32> = digest.iter().map(|&b| b as u32).collect();
    let mut digits = Vec::with_capacity(88);
    for _ in 0..88 {
        // num /= 58, remainder is the next base-58 digit (least significant first).
        let mut rem = 0u32;
        for byte in num.iter_mut() {
            let cur = (rem << 8) | *byte;
            *byte = cur / 58;
            rem = cur % 58;
        }
        digits.push(C4_ALPHABET[rem as usize]);
    }
    digits.reverse();
    format!(
        "c4{}",
        String::from_utf8(digits).expect("alphabet is ASCII")
    )
}

// ------------------------------------------------------ directory hashes --

fn digest_of(algo: Algo, data: &[u8]) -> String {
    let mut h = super::hash::MultiHasher::new(&[algo]);
    h.update(data);
    h.finish()
        .into_iter()
        .find(|(a, _)| *a == algo)
        .map(|(_, d)| d)
        .expect("requested algo is produced")
}

fn hex_bytes(s: &str) -> Vec<u8> {
    (0..s.len() / 2)
        .filter_map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).ok())
        .collect()
}

/// The reference tool's "hash of hash list": sort the hex strings, hash the
/// concatenation of their bytes. An empty list hashes nothing.
fn hash_of_list(algo: Algo, mut list: Vec<String>) -> String {
    list.sort();
    let mut bytes = Vec::new();
    for h in &list {
        bytes.extend(hex_bytes(h));
    }
    digest_of(algo, &bytes)
}

#[derive(Default)]
struct DirAcc {
    content: Vec<String>,
    structure: Vec<String>,
}

pub struct DirHash {
    pub rel: String,
    pub content: String,
    pub structure: String,
}

/// Content and structure hashes for every folder (and the root, `rel == ""`),
/// computed bottom-up from the file hashes exactly as `ascmhl` does:
/// content = hash of the sorted child content hashes; structure = hash of
/// the sorted `hash(name + child hash bytes)` values.
pub fn dir_hashes(algo: Algo, records: &[Record], empty_dirs: &[String]) -> Vec<DirHash> {
    let mut dirs: BTreeMap<String, DirAcc> = BTreeMap::new();
    dirs.entry(String::new()).or_default();
    let parent_of = |rel: &str| {
        rel.rsplit_once('/')
            .map(|(p, _)| p.to_string())
            .unwrap_or_default()
    };
    let name_of = |rel: &str| rel.rsplit('/').next().unwrap_or(rel).to_string();
    // Register every folder on every path so empty intermediate levels exist.
    let register = |rel: &str, dirs: &mut BTreeMap<String, DirAcc>| {
        let mut cur = rel.to_string();
        while !cur.is_empty() {
            dirs.entry(cur.clone()).or_default();
            cur = parent_of(&cur);
        }
    };
    for r in records {
        register(&parent_of(&r.rel), &mut dirs);
    }
    for d in empty_dirs {
        register(d, &mut dirs);
    }
    for r in records {
        let Some(h) = r.hash(algo) else { continue };
        let acc = dirs.entry(parent_of(&r.rel)).or_default();
        acc.content.push(h.to_string());
        let mut s = name_of(&r.rel).into_bytes();
        s.extend(hex_bytes(h));
        acc.structure.push(digest_of(algo, &s));
    }
    // Deepest folders first, so each folder is final before its parent reads it.
    let mut order: Vec<String> = dirs.keys().cloned().collect();
    order.sort_by_key(|d| {
        std::cmp::Reverse(if d.is_empty() {
            0
        } else {
            d.matches('/').count() + 1
        })
    });
    let mut out = Vec::new();
    for d in order {
        let acc = dirs.remove(&d).unwrap_or_default();
        let content = hash_of_list(algo, acc.content);
        let structure = hash_of_list(algo, acc.structure);
        if !d.is_empty() {
            let parent = dirs.entry(parent_of(&d)).or_default();
            parent.content.push(content.clone());
            let mut s = name_of(&d).into_bytes();
            s.extend(hex_bytes(&structure));
            parent.structure.push(digest_of(algo, &s));
        }
        out.push(DirHash {
            rel: d,
            content,
            structure,
        });
    }
    out
}

// --------------------------------------------------------- writing MHL --

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn local_iso(t: SystemTime) -> String {
    let dt: DateTime<Local> = t.into();
    dt.format("%Y-%m-%dT%H:%M:%S%:z").to_string()
}

fn hostname() -> String {
    for key in ["COMPUTERNAME", "HOSTNAME"] {
        if let Ok(v) = std::env::var(key)
            && !v.trim().is_empty()
        {
            return v.trim().to_string();
        }
    }
    std::process::Command::new("hostname")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_string())
}

/// Patterns the generation tells verifiers to ignore: OS clutter and our own
/// bookkeeping, so `ascmhl verify` never calls them new or missing.
const IGNORE: &[&str] = &[
    ".DS_Store",
    "ascmhl",
    "ascmhl/",
    "_DIT",
    "_DIT/",
    "*.nolgia-part",
    "Thumbs.db",
    "desktop.ini",
    "System Volume Information",
    "$RECYCLE.BIN",
    ".Spotlight-V100",
    ".Trashes",
    ".fseventsd",
];

pub enum Process {
    InPlace,
    Transfer,
}

/// One file in a generation: its hashes, the `action` they are recorded
/// with, and the name it had before a rename (`<previousPath>`).
pub struct MhlEntry {
    pub record: Record,
    pub action: &'static str,
    pub previous: Option<String>,
}

/// Append a new generation to `root/ascmhl/` and update the chain. Records
/// carry `action="original"` the first time a file is hashed and
/// `"verified"` (or `"failed"`) on a later check, as the spec defines.
pub fn write_mhl_generation(
    root: &Path,
    records: &[Record],
    empty_dirs: &[String],
    process: Process,
    action: &'static str,
) -> Result<PathBuf> {
    let entries: Vec<MhlEntry> = records
        .iter()
        .map(|r| MhlEntry {
            record: r.clone(),
            action,
            previous: None,
        })
        .collect();
    write_mhl_entries(root, &entries, empty_dirs, process)
}

/// [`write_mhl_generation`] with an action per file, and the previous name
/// of each renamed file, as the reference `ascmhl create --detect_renaming`
/// writes them (a renamed file is `original` under its new name).
pub fn write_mhl_entries(
    root: &Path,
    entries: &[MhlEntry],
    empty_dirs: &[String],
    process: Process,
) -> Result<PathBuf> {
    let records: Vec<Record> = entries.iter().map(|e| e.record.clone()).collect();
    let records = records.as_slice();
    let mhl_dir = root.join(MHL_DIR);
    fs::create_dir_all(&mhl_dir)
        .with_context(|| format!("could not create {}", mhl_dir.display()))?;
    let chain = read_chain(&mhl_dir)?;
    let seq = chain.iter().map(|(n, _)| *n).max().unwrap_or(0) + 1;
    let root_name = root
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "root".to_string());
    let now = Utc::now();
    let file_name = format!(
        "{seq:04}_{}_{}.mhl",
        root_name,
        now.format("%Y-%m-%d_%H%M%SZ")
    );
    let hashdate = Local::now().format("%Y-%m-%dT%H:%M:%S%.6f%:z").to_string();
    let created = Local::now().format("%Y-%m-%dT%H:%M:%S%:z").to_string();

    let mut x = String::new();
    x.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    x.push_str("<hashlist version=\"2.0\" xmlns=\"urn:ASC:MHL:v2.0\">\n");
    x.push_str("  <creatorinfo>\n");
    x.push_str(&format!("    <creationdate>{created}</creationdate>\n"));
    x.push_str(&format!(
        "    <hostname>{}</hostname>\n",
        xml_escape(&hostname())
    ));
    x.push_str(&format!(
        "    <tool version=\"{}\">nolgia dit</tool>\n",
        env!("CARGO_PKG_VERSION")
    ));
    x.push_str("  </creatorinfo>\n");
    x.push_str("  <processinfo>\n");
    x.push_str(&format!(
        "    <process>{}</process>\n",
        match process {
            Process::InPlace => "in-place",
            Process::Transfer => "transfer",
        }
    ));
    let dirs = dir_hashes(Algo::Xxh64, records, empty_dirs);
    if let Some(root_hash) = dirs.iter().find(|d| d.rel.is_empty()) {
        x.push_str("    <roothash>\n");
        x.push_str(&format!(
            "      <content>\n        <xxh64 hashdate=\"{hashdate}\">{}</xxh64>\n      </content>\n",
            root_hash.content
        ));
        x.push_str(&format!(
            "      <structure>\n        <xxh64 hashdate=\"{hashdate}\">{}</xxh64>\n      </structure>\n",
            root_hash.structure
        ));
        x.push_str("    </roothash>\n");
    }
    x.push_str("    <ignore>\n");
    for p in IGNORE {
        x.push_str(&format!("      <pattern>{}</pattern>\n", xml_escape(p)));
    }
    x.push_str("    </ignore>\n");
    x.push_str("  </processinfo>\n");
    x.push_str("  <hashes>\n");
    let mut sorted: Vec<&MhlEntry> = entries.iter().collect();
    sorted.sort_by(|a, b| a.record.rel.cmp(&b.record.rel));
    for e in sorted {
        let r = &e.record;
        let action = e.action;
        let mtime = r.mtime.map(local_iso).unwrap_or_else(|| created.clone());
        x.push_str("    <hash>\n");
        x.push_str(&format!(
            "      <path size=\"{}\" lastmodificationdate=\"{mtime}\">{}</path>\n",
            r.size,
            xml_escape(&r.rel)
        ));
        for (algo, digest) in &r.hashes {
            x.push_str(&format!(
                "      <{n} action=\"{action}\" hashdate=\"{hashdate}\">{digest}</{n}>\n",
                n = algo.name()
            ));
        }
        if let Some(p) = &e.previous {
            x.push_str(&format!(
                "      <previousPath>{}</previousPath>\n",
                xml_escape(p)
            ));
        }
        x.push_str("    </hash>\n");
    }
    for d in dirs.iter().filter(|d| !d.rel.is_empty()) {
        let mdate = fs::metadata(root.join(&d.rel))
            .and_then(|m| m.modified())
            .map(local_iso)
            .unwrap_or_else(|_| created.clone());
        x.push_str("    <directoryhash>\n");
        x.push_str(&format!(
            "      <path lastmodificationdate=\"{mdate}\">{}</path>\n",
            xml_escape(&d.rel)
        ));
        x.push_str(&format!(
            "      <content>\n        <xxh64 hashdate=\"{hashdate}\">{}</xxh64>\n      </content>\n",
            d.content
        ));
        x.push_str(&format!(
            "      <structure>\n        <xxh64 hashdate=\"{hashdate}\">{}</xxh64>\n      </structure>\n",
            d.structure
        ));
        x.push_str("    </directoryhash>\n");
    }
    x.push_str("  </hashes>\n</hashlist>\n");

    let path = mhl_dir.join(&file_name);
    write_synced(&path, x.as_bytes())?;
    let mut chain = chain;
    chain.push((seq, (file_name, c4_id(x.as_bytes()))));
    let mut c = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<ascmhldirectory xmlns=\"urn:ASC:MHL:DIRECTORY:v2.0\">\n",
    );
    for (n, (name, c4)) in &chain {
        c.push_str(&format!(
            "  <hashlist sequencenr=\"{n}\">\n    <path>{}</path>\n    <c4>{c4}</c4>\n  </hashlist>\n",
            xml_escape(name)
        ));
    }
    c.push_str("</ascmhldirectory>\n");
    write_synced(&mhl_dir.join("ascmhl_chain.xml"), c.as_bytes())?;
    Ok(path)
}

/// Write through a temp name and rename, so a crash never leaves half a file.
pub fn write_synced(path: &Path, bytes: &[u8]) -> Result<()> {
    let tmp = path.with_extension(format!(
        "{}{PART_SUFFIX}",
        path.extension()
            .map(|e| e.to_string_lossy().to_string())
            .unwrap_or_default()
    ));
    {
        let mut f =
            fs::File::create(&tmp).with_context(|| format!("could not write {}", tmp.display()))?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    fs::rename(&tmp, path).with_context(|| format!("could not finish {}", path.display()))?;
    Ok(())
}

// --------------------------------------------------------- reading MHL --

type Chain = Vec<(u32, (String, String))>;

fn read_chain(mhl_dir: &Path) -> Result<Chain> {
    let path = mhl_dir.join("ascmhl_chain.xml");
    let Ok(text) = fs::read_to_string(&path) else {
        return Ok(Vec::new());
    };
    let mut reader = Reader::from_str(&text);
    let mut out = Vec::new();
    let (mut seq, mut name, mut c4) = (0u32, String::new(), String::new());
    let mut cur = String::new();
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => {
                cur = String::from_utf8_lossy(e.local_name().as_ref()).to_string();
                if cur == "hashlist" {
                    for a in e.attributes().flatten() {
                        if a.key.local_name().as_ref() == b"sequencenr" {
                            seq = String::from_utf8_lossy(&a.value).parse().unwrap_or(0);
                        }
                    }
                }
            }
            Ok(Event::Text(t)) => {
                let v = t.unescape().map(|s| s.to_string()).unwrap_or_default();
                match cur.as_str() {
                    "path" => name = v,
                    "c4" => c4 = v,
                    _ => {}
                }
            }
            Ok(Event::End(e)) => {
                if e.local_name().as_ref() == b"hashlist" {
                    out.push((seq, (std::mem::take(&mut name), std::mem::take(&mut c4))));
                }
                cur.clear();
            }
            Ok(Event::Eof) => break,
            Err(e) => bail!("could not read {}: {e}", path.display()),
            _ => {}
        }
    }
    Ok(out)
}

/// Every file record in an ASC MHL history, oldest generation first, so a
/// later generation's hash for a path wins. Each generation's bytes are
/// checked against the C4 ID in the chain; a mismatch means the manifest
/// itself was edited and is reported, not trusted.
pub fn read_mhl_history(root: &Path) -> Result<(BTreeMap<String, Record>, Vec<String>)> {
    let mhl_dir = root.join(MHL_DIR);
    let chain = read_chain(&mhl_dir)?;
    let mut warnings = Vec::new();
    let mut names: Vec<(u32, String)> = chain
        .iter()
        .map(|(n, (name, _))| (*n, name.clone()))
        .collect();
    if names.is_empty() {
        // No chain: take every .mhl in name order (their names start with the sequence number).
        let mut found: Vec<String> = fs::read_dir(&mhl_dir)
            .with_context(|| format!("no ASC MHL folder at {}", mhl_dir.display()))?
            .flatten()
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|n| n.to_ascii_lowercase().ends_with(".mhl"))
            .collect();
        found.sort();
        names = found
            .into_iter()
            .enumerate()
            .map(|(i, n)| (i as u32 + 1, n))
            .collect();
    }
    names.sort();
    let c4s: HashMap<String, String> = chain.into_iter().map(|(_, (n, c))| (n, c)).collect();
    let mut out = BTreeMap::new();
    for (_, name) in names {
        let path = mhl_dir.join(&name);
        let bytes =
            fs::read(&path).with_context(|| format!("could not read {}", path.display()))?;
        if let Some(expected) = c4s.get(&name)
            && &c4_id(&bytes) != expected
        {
            warnings.push(format!(
                    "{name}: its C4 ID does not match ascmhl_chain.xml, so the manifest file was changed after it was written"
                ));
        }
        for (r, action, previous) in parse_generation(&String::from_utf8_lossy(&bytes))? {
            // A renamed file: its old name is no longer expected.
            if let Some(p) = previous
                && action != "failed"
            {
                out.remove(&p);
            }
            // The trusted hash is the one first recorded as `original`. A
            // later `verified` restates it; a later `failed` is evidence the
            // file went bad and must never become the new reference.
            match (out.contains_key(&r.rel), action.as_str()) {
                (_, "failed") => {
                    warnings.push(format!("{name} recorded a failed check for {}", r.rel));
                    out.entry(r.rel.clone()).or_insert(r);
                }
                (false, _) | (true, "original") => {
                    out.insert(r.rel.clone(), r);
                }
                _ => {}
            }
        }
    }
    Ok((out, warnings))
}

/// Each file record with the `action` its hashes were recorded with and,
/// for a renamed file, its previous name.
#[allow(clippy::type_complexity)]
fn parse_generation(text: &str) -> Result<Vec<(Record, String, Option<String>)>> {
    let mut reader = Reader::from_str(text);
    let mut out = Vec::new();
    let mut in_hash = false;
    let mut cur = String::new();
    let mut rec = Record {
        rel: String::new(),
        size: 0,
        mtime: None,
        hashes: Vec::new(),
    };
    let mut action = String::new();
    let mut previous: Option<String> = None;
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => {
                cur = String::from_utf8_lossy(e.local_name().as_ref()).to_string();
                if cur == "hash" {
                    in_hash = true;
                    rec = Record {
                        rel: String::new(),
                        size: 0,
                        mtime: None,
                        hashes: Vec::new(),
                    };
                    action = "original".to_string();
                } else if in_hash && Algo::parse(&cur).is_some() {
                    for a in e.attributes().flatten() {
                        if a.key.local_name().as_ref() == b"action"
                            && String::from_utf8_lossy(&a.value) == "failed"
                        {
                            action = "failed".to_string();
                        } else if a.key.local_name().as_ref() == b"action" && action != "failed" {
                            action = String::from_utf8_lossy(&a.value).to_string();
                        }
                    }
                } else if in_hash && cur == "path" {
                    for a in e.attributes().flatten() {
                        if a.key.local_name().as_ref() == b"size" {
                            rec.size = String::from_utf8_lossy(&a.value).parse().unwrap_or(0);
                        }
                    }
                }
            }
            Ok(Event::Text(t)) if in_hash => {
                let v = t.unescape().map(|s| s.to_string()).unwrap_or_default();
                if cur == "path" {
                    rec.rel = v;
                } else if cur == "previousPath" {
                    previous = Some(v);
                } else if let Some(algo) = Algo::parse(&cur) {
                    let v = v.trim().to_ascii_lowercase();
                    rec.hashes.retain(|(a, _)| *a != algo);
                    rec.hashes.push((algo, v));
                }
            }
            Ok(Event::End(e)) => {
                if e.local_name().as_ref() == b"hash" {
                    in_hash = false;
                    rec.hashes.sort_by_key(|(a, _)| *a);
                    let done = std::mem::replace(
                        &mut rec,
                        Record {
                            rel: String::new(),
                            size: 0,
                            mtime: None,
                            hashes: Vec::new(),
                        },
                    );
                    out.push((done, std::mem::take(&mut action), previous.take()));
                }
                cur.clear();
            }
            Ok(Event::Eof) => break,
            Err(e) => bail!("could not read an ASC MHL generation: {e}"),
            _ => {}
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------- TSV --

/// `# comment` lines, then a header naming the columns: one per hash
/// (`xxh64`, `md5`, `sha1`), then `bytes`, `mtime`, `relative_path`. This is
/// also the format the hand-made manifests before this command used, so
/// those verify too.
pub fn write_tsv(path: &Path, root_label: &str, records: &[Record], algos: &[Algo]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut s = format!(
        "# nolgia dit manifest of {root_label}, made {}\n",
        Local::now().format("%Y-%m-%dT%H:%M:%S%:z")
    );
    let mut header: Vec<&str> = algos.iter().map(|a| a.name()).collect();
    header.extend(["bytes", "mtime", "relative_path"]);
    s.push_str(&header.join("\t"));
    s.push('\n');
    let mut sorted: Vec<&Record> = records.iter().collect();
    sorted.sort_by(|a, b| a.rel.cmp(&b.rel));
    for r in sorted {
        let mut row: Vec<String> = algos
            .iter()
            .map(|a| r.hash(*a).unwrap_or("").to_string())
            .collect();
        row.push(r.size.to_string());
        row.push(
            r.mtime
                .map(|t| {
                    DateTime::<Local>::from(t)
                        .format("%Y-%m-%dT%H:%M:%S")
                        .to_string()
                })
                .unwrap_or_default(),
        );
        row.push(r.rel.clone());
        s.push_str(&row.join("\t"));
        s.push('\n');
    }
    write_synced(path, s.as_bytes())
}

pub fn read_tsv(path: &Path) -> Result<BTreeMap<String, Record>> {
    let text =
        fs::read_to_string(path).with_context(|| format!("could not read {}", path.display()))?;
    let mut header: Option<Vec<String>> = None;
    let mut out = BTreeMap::new();
    for line in text.lines() {
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        let cols: Vec<&str> = line.split('\t').collect();
        let Some(h) = header.as_ref() else {
            header = Some(cols.iter().map(|c| c.trim().to_ascii_lowercase()).collect());
            continue;
        };
        let mut rec = Record {
            rel: String::new(),
            size: 0,
            mtime: None,
            hashes: Vec::new(),
        };
        for (name, value) in h.iter().zip(cols.iter()) {
            match name.as_str() {
                "bytes" | "size" => rec.size = value.parse().unwrap_or(0),
                "relative_path" | "path" => rec.rel = value.replace('\\', "/"),
                other => {
                    if let Some(algo) = Algo::parse(other)
                        && !value.is_empty()
                    {
                        rec.hashes.push((algo, value.trim().to_ascii_lowercase()));
                    }
                }
            }
        }
        if rec.rel.is_empty() {
            continue;
        }
        rec.hashes.sort_by_key(|(a, _)| *a);
        out.insert(rec.rel.clone(), rec);
    }
    if header.is_none() {
        bail!(
            "{} has no header line (expected columns like xxh64, bytes, mtime, relative_path)",
            path.display()
        );
    }
    Ok(out)
}

/// Where a folder's manifests live: ASC MHL inside it, the TSV in `_DIT`.
pub fn tsv_path_for(root: &Path) -> PathBuf {
    root.join(DIT_DIR).join("manifest.tsv")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn c4_matches_the_reference_vectors() {
        assert_eq!(
            c4_id(b""),
            "c459dsjfscH38cYeXXYogktxf4Cd9ibshE3BHUo6a58hBXmRQdZrAkZzsWcbWtDg5oQstpDuni4Hirj75GEmTc1sFT"
        );
        assert_eq!(
            c4_id(b"foo"),
            "c45xZeXwMSpqXjpDumcHMA6mhoAmGHkUo7r9WmN2UgSEQzj9KjgseaQdkEJ11fGb5S1WEENcV3q8RFWwEeVpC7Fjk2"
        );
    }

    fn rec(rel: &str, data: &[u8]) -> Record {
        let mut h = super::super::hash::MultiHasher::new(&[Algo::Xxh64]);
        h.update(data);
        Record {
            rel: rel.into(),
            size: data.len() as u64,
            mtime: None,
            hashes: h.finish(),
        }
    }

    #[test]
    fn directory_hashes_match_the_reference_tool() {
        // Values written by `ascmhl create -h xxh64` (ascmhl 1.0.1) for:
        // CLIP/C0001.MP4 = "hello", a.txt = "x", EMPTY/ (empty folder).
        let records = vec![rec("CLIP/C0001.MP4", b"hello"), rec("a.txt", b"x")];
        assert_eq!(records[0].hash(Algo::Xxh64), Some("26c7827d889f6da3"));
        let dirs = dir_hashes(Algo::Xxh64, &records, &["EMPTY".to_string()]);
        let get = |rel: &str| dirs.iter().find(|d| d.rel == rel).unwrap();
        assert_eq!(get("CLIP").content, "c07c10338a825a5d");
        assert_eq!(get("CLIP").structure, "7977796842ee1ce2");
        assert_eq!(get("EMPTY").content, "ef46db3751d8e999");
        assert_eq!(get("").content, "e24458f37f197847");
        assert_eq!(get("").structure, "3ff34a14d1e603e3");
    }

    #[test]
    fn a_failed_check_never_becomes_the_reference_hash() {
        // Found against the reference tool: after a corrupted file, `ascmhl
        // create` appends a generation with the NEW hash and action="failed".
        let dir = tempfile::tempdir().unwrap();
        let mhl = dir.path().join("ascmhl");
        fs::create_dir_all(&mhl).unwrap();
        let generation = |action: &str, hash: &str| {
            format!(
                "<hashlist version=\"2.0\" xmlns=\"urn:ASC:MHL:v2.0\"><hashes><hash>\
                 <path size=\"5\">a.MP4</path><xxh64 action=\"{action}\">{hash}</xxh64>\
                 </hash></hashes></hashlist>"
            )
        };
        fs::write(
            mhl.join("0001_x.mhl"),
            generation("original", "26c7827d889f6da3"),
        )
        .unwrap();
        fs::write(
            mhl.join("0002_x.mhl"),
            generation("failed", "ffffffffffffffff"),
        )
        .unwrap();
        let (recs, warnings) = read_mhl_history(dir.path()).unwrap();
        assert_eq!(recs["a.MP4"].hash(Algo::Xxh64), Some("26c7827d889f6da3"));
        assert!(warnings[0].contains("failed check"));
    }

    #[test]
    fn tsv_round_trip_and_legacy_header() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("m.tsv");
        let records = vec![rec("a/b.MP4", b"hello")];
        write_tsv(&p, "x", &records, &[Algo::Xxh64]).unwrap();
        let back = read_tsv(&p).unwrap();
        assert_eq!(back["a/b.MP4"].hash(Algo::Xxh64), Some("26c7827d889f6da3"));
        // The hand-made manifest format from before this command existed.
        fs::write(&p, "# xxh64 manifest of E:\\X\nxxh64\tbytes\tmtime\trelative_path\n26c7827d889f6da3\t5\t2026-10-12T13:01:25\tPRIVATE\\C0001.MP4\n").unwrap();
        let legacy = read_tsv(&p).unwrap();
        assert_eq!(legacy["PRIVATE/C0001.MP4"].size, 5);
    }
}
