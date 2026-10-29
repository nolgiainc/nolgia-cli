//! Walking a card or folder: every regular file with its size and mtime,
//! relative paths with `/` separators (the form manifests store on every
//! platform), and the operating-system clutter a card collects skipped and
//! listed rather than silently copied or silently dropped.

use std::{
    fs,
    path::{Path, PathBuf},
    time::SystemTime,
};

use anyhow::{Context, Result};

/// Folders and files an OS writes onto removable media. They are never part
/// of a shoot, copying them can fail (permissions), and their contents
/// change on their own, which would break verification. Matched on the path
/// component, case-insensitively.
const SKIP_NAMES: &[&str] = &[
    "system volume information",
    "$recycle.bin",
    ".spotlight-v100",
    ".trashes",
    ".fseventsd",
    ".temporaryitems",
    ".ds_store",
    "thumbs.db",
    "desktop.ini",
];

/// Our own bookkeeping, kept next to the media but never treated as media.
pub const DIT_DIR: &str = "_DIT";
pub const MHL_DIR: &str = "ascmhl";
pub const PART_SUFFIX: &str = ".nolgia-part";

#[derive(Clone, Debug)]
pub struct Entry {
    /// Path relative to the walked root, `/`-separated.
    pub rel: String,
    pub abs: PathBuf,
    pub size: u64,
    pub mtime: Option<SystemTime>,
}

pub struct Walk {
    pub files: Vec<Entry>,
    /// Folders with nothing in them (card structure such as `M4ROOT/SUB`),
    /// recreated at the destination so a copied card keeps its shape.
    pub empty_dirs: Vec<String>,
    /// OS clutter that was skipped, for the report.
    pub skipped: Vec<String>,
}

pub fn is_skipped_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    SKIP_NAMES.contains(&lower.as_str()) || lower.starts_with("._")
}

/// Is this our own bookkeeping (never media, never in a manifest)?
pub fn is_dit_internal(rel: &str) -> bool {
    rel.split('/')
        .any(|part| part.eq_ignore_ascii_case(MHL_DIR) || part.eq_ignore_ascii_case(DIT_DIR))
        || rel.ends_with(PART_SUFFIX)
}

pub fn walk(root: &Path) -> Result<Walk> {
    let mut out = Walk {
        files: Vec::new(),
        empty_dirs: Vec::new(),
        skipped: Vec::new(),
    };
    walk_into(root, root, &mut out)?;
    out.files.sort_by(|a, b| a.rel.cmp(&b.rel));
    out.empty_dirs.sort();
    out.skipped.sort();
    Ok(out)
}

fn walk_into(root: &Path, dir: &Path, out: &mut Walk) -> Result<()> {
    let mut entries: Vec<_> = fs::read_dir(dir)
        .with_context(|| format!("could not read the folder {}", dir.display()))?
        .collect::<std::io::Result<_>>()
        .with_context(|| format!("could not list the folder {}", dir.display()))?;
    entries.sort_by_key(|e| e.file_name());
    let mut kept = 0usize;
    for entry in entries {
        let name = entry.file_name().to_string_lossy().to_string();
        let path = entry.path();
        let rel = rel_path(root, &path);
        if is_skipped_name(&name) {
            out.skipped.push(rel);
            continue;
        }
        if is_dit_internal(&rel) {
            continue;
        }
        let meta = fs::symlink_metadata(&path)
            .with_context(|| format!("could not read {}", path.display()))?;
        if meta.is_dir() {
            kept += 1;
            walk_into(root, &path, out)?;
        } else if meta.is_file() {
            kept += 1;
            out.files.push(Entry {
                rel,
                abs: path,
                size: meta.len(),
                mtime: meta.modified().ok(),
            });
        } else {
            // Symlinks and devices are not camera media; say so in the report.
            out.skipped.push(rel);
        }
    }
    if kept == 0 && dir != root {
        out.empty_dirs.push(rel_path(root, dir));
    }
    Ok(())
}

/// Folders a camera writes at the root of its card (matched case-insensitively):
/// DCIM (stills cameras, mirrorless, drones, GoPro), PRIVATE (Sony M4ROOT,
/// AVCHD), XDROOT and M4ROOT (Sony), AVCHD, CONTENTS (Panasonic P2, Canon
/// XF-AVC) and AVF_INFO (Panasonic). A RED card's `.RDM` folder counts too.
const CARD_ROOT_FOLDERS: &[&str] = &[
    "dcim", "private", "xdroot", "m4root", "avchd", "contents", "avf_info",
];

/// Does `dir` look like the root of a camera card?
pub fn is_card_root(dir: &Path) -> bool {
    let Ok(entries) = fs::read_dir(dir) else {
        return false;
    };
    entries.flatten().any(|e| {
        let name = e.file_name().to_string_lossy().to_ascii_lowercase();
        e.path().is_dir() && (CARD_ROOT_FOLDERS.contains(&name.as_str()) || name.ends_with(".rdm"))
    })
}

/// What is directly inside a folder that is not a card itself: the card
/// roots, and everything else (OS clutter and NOLGIA's own `_DIT` and
/// `ascmhl` aside).
#[derive(Debug, Default)]
pub struct CardsInside {
    pub cards: Vec<PathBuf>,
    pub others: Vec<String>,
}

pub fn cards_inside(dir: &Path) -> Result<CardsInside> {
    let mut out = CardsInside::default();
    let mut entries: Vec<_> = fs::read_dir(dir)
        .with_context(|| format!("could not read the folder {}", dir.display()))?
        .flatten()
        .collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let name = e.file_name().to_string_lossy().to_string();
        if is_skipped_name(&name) || is_dit_internal(&name) {
            continue;
        }
        let path = e.path();
        if path.is_dir() && is_card_root(&path) {
            out.cards.push(path);
        } else {
            out.others.push(name);
        }
    }
    Ok(out)
}

/// Reserve a file name that nothing has used yet: `path` itself, else
/// `<stem>-2.<ext>`, `<stem>-3.<ext>` and so on. The file is created empty
/// (create_new, so two runs in the same second can never get the same name)
/// and none of the `siblings` extensions may exist beside it either (a
/// report's `.json` beside its `.md`). DIT records are never overwritten.
pub fn reserve_unique(path: &Path, siblings: &[&str]) -> Result<PathBuf> {
    let dir = path.parent().map(PathBuf::from).unwrap_or_default();
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let ext = path
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    for n in 1..10_000u32 {
        let candidate = if n == 1 {
            path.to_path_buf()
        } else {
            dir.join(format!("{stem}-{n}{ext}"))
        };
        if siblings
            .iter()
            .any(|e| candidate.with_extension(e).exists())
        {
            continue;
        }
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate)
        {
            Ok(_) => return Ok(candidate),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => {
                return Err(e).with_context(|| format!("could not create {}", candidate.display()));
            }
        }
    }
    anyhow::bail!("could not find a free name for {}", path.display())
}

/// `root/a/b.MP4` -> `a/b.MP4` with `/` separators.
pub fn rel_path(root: &Path, path: &Path) -> String {
    let rel = path.strip_prefix(root).unwrap_or(path);
    rel.components()
        .map(|c| c.as_os_str().to_string_lossy().to_string())
        .collect::<Vec<_>>()
        .join("/")
}

/// The native path for a manifest-relative path under `root`.
pub fn join_rel(root: &Path, rel: &str) -> PathBuf {
    let mut p = root.to_path_buf();
    for part in rel.split(['/', '\\']).filter(|s| !s.is_empty()) {
        p.push(part);
    }
    long_path(p)
}

/// Windows refuses paths over 260 characters unless they carry the `\\?\`
/// prefix; deep card structures plus a long shoot folder can get there.
#[cfg(windows)]
pub fn long_path(p: PathBuf) -> PathBuf {
    let s = p.to_string_lossy();
    if s.len() < 240 || s.starts_with(r"\\?\") {
        return p;
    }
    match std::path::absolute(&p) {
        Ok(abs) => {
            let a = abs.to_string_lossy().to_string();
            if let Some(unc) = a.strip_prefix(r"\\") {
                PathBuf::from(format!(r"\\?\UNC\{unc}"))
            } else {
                PathBuf::from(format!(r"\\?\{a}"))
            }
        }
        Err(_) => p,
    }
}

#[cfg(not(windows))]
pub fn long_path(p: PathBuf) -> PathBuf {
    p
}

/// A folder name for a card when none is given: the source folder's own name,
/// or for a drive root (`E:\`, `/Volumes/X` already has a name) `CARD_E`.
pub fn default_card_name(src: &Path, index: usize) -> String {
    let s = src.to_string_lossy();
    let bytes = s.as_bytes();
    // A Windows drive root: `E:`, `E:\` or `E:/`.
    if (bytes.len() == 2 || (bytes.len() == 3 && (bytes[2] == b'\\' || bytes[2] == b'/')))
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
    {
        return format!("CARD_{}", (bytes[0] as char).to_ascii_uppercase());
    }
    if let Some(name) = src.file_name() {
        let n = sanitize_component(&name.to_string_lossy());
        if !n.is_empty() {
            return n;
        }
    }
    format!("CARD{:02}", index + 1)
}

/// Characters no filesystem we target accepts in a name.
pub fn sanitize_component(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') || c.is_control() {
                '_'
            } else {
                c
            }
        })
        .collect();
    cleaned.trim().trim_end_matches('.').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rel_paths_use_forward_slashes() {
        let root = Path::new("root");
        let p = root
            .join("PRIVATE")
            .join("M4ROOT")
            .join("CLIP")
            .join("C0001.MP4");
        assert_eq!(rel_path(root, &p), "PRIVATE/M4ROOT/CLIP/C0001.MP4");
        assert_eq!(join_rel(root, "a/b.txt"), root.join("a").join("b.txt"));
    }

    #[test]
    fn os_clutter_is_skipped_and_our_files_are_internal() {
        assert!(is_skipped_name("System Volume Information"));
        assert!(is_skipped_name("._C0001.MP4"));
        assert!(!is_skipped_name("C0001.MP4"));
        assert!(is_dit_internal("ascmhl/0001_x.mhl"));
        assert!(is_dit_internal("_DIT/report.md"));
        assert!(is_dit_internal("A001/ascmhl/ascmhl_chain.xml"));
        assert!(is_dit_internal("a/C0001.MP4.nolgia-part"));
        assert!(!is_dit_internal("PRIVATE/C0001.MP4"));
    }

    #[test]
    fn reserve_unique_never_reuses_a_name() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("DIT-REPORT-20261029-114410.md");
        let a = reserve_unique(&p, &["json"]).unwrap();
        assert_eq!(a, p);
        fs::write(&a, "offload report").unwrap();
        let b = reserve_unique(&p, &["json"]).unwrap();
        assert_eq!(b.file_name().unwrap(), "DIT-REPORT-20261029-114410-2.md");
        // A stray .json beside a free .md name also moves the name on.
        fs::write(tmp.path().join("DIT-REPORT-20261029-114410-3.json"), "{}").unwrap();
        let c = reserve_unique(&p, &["json"]).unwrap();
        assert_eq!(c.file_name().unwrap(), "DIT-REPORT-20261029-114410-4.md");
        assert_eq!(fs::read_to_string(&a).unwrap(), "offload report");
    }

    #[test]
    fn card_roots_are_found_inside_a_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let top = tmp.path().join("cards");
        fs::create_dir_all(top.join("A001/PRIVATE/M4ROOT/CLIP")).unwrap();
        fs::create_dir_all(top.join("B001/DCIM/100EOSR6")).unwrap();
        fs::create_dir_all(top.join("_DIT")).unwrap();
        fs::write(top.join(".DS_Store"), "x").unwrap();
        assert!(!is_card_root(&top));
        assert!(is_card_root(&top.join("A001")));
        let inside = cards_inside(&top).unwrap();
        let names: Vec<_> = inside
            .cards
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        assert_eq!(names, ["A001", "B001"]);
        assert!(inside.others.is_empty());
        fs::write(top.join("notes.txt"), "x").unwrap();
        assert_eq!(cards_inside(&top).unwrap().others, ["notes.txt"]);
        fs::create_dir_all(tmp.path().join("R/A001_C001.RDM")).unwrap();
        assert!(is_card_root(&tmp.path().join("R")));
    }

    #[test]
    fn card_names() {
        assert_eq!(default_card_name(Path::new("/Volumes/A001"), 0), "A001");
        assert_eq!(default_card_name(Path::new("E:\\"), 0), "CARD_E");
        assert_eq!(sanitize_component("a:b?c."), "a_b_c");
    }
}
