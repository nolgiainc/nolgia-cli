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
    fn card_names() {
        assert_eq!(default_card_name(Path::new("/Volumes/A001"), 0), "A001");
        assert_eq!(default_card_name(Path::new("E:\\"), 0), "CARD_E");
        assert_eq!(sanitize_component("a:b?c."), "a_b_c");
    }
}
