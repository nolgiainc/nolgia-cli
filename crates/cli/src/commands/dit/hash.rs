//! Content hashes for `nolgia dit`.
//!
//! xxh64 is the default: it is what ASC MHL tools (Silverstack, ShotPut,
//! Hedge, the reference `ascmhl`) write by default and it runs at disk speed.
//! MD5 and SHA-1 exist because some post houses and studios still ask for
//! them in delivery specs; any combination can be computed in the same read.

use anyhow::{Result, bail};
use md5::{Digest as _, Md5};
use sha1::Sha1;
use xxhash_rust::xxh64::Xxh64;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Ord, PartialOrd)]
pub enum Algo {
    Xxh64,
    Md5,
    Sha1,
}

impl Algo {
    /// The element / column name, as ASC MHL spells it.
    pub fn name(self) -> &'static str {
        match self {
            Algo::Xxh64 => "xxh64",
            Algo::Md5 => "md5",
            Algo::Sha1 => "sha1",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "xxh64" | "xxhash64" | "xxhash" => Some(Algo::Xxh64),
            "md5" => Some(Algo::Md5),
            "sha1" | "sha-1" => Some(Algo::Sha1),
            _ => None,
        }
    }

    /// `xxh64,md5` -> [Xxh64, Md5]; xxh64 always comes first and is always
    /// present, because it is the hash the manifests and verify key on.
    pub fn parse_list(list: &str) -> Result<Vec<Algo>> {
        let mut algos = vec![Algo::Xxh64];
        for part in list.split(',').filter(|p| !p.trim().is_empty()) {
            let Some(algo) = Algo::parse(part) else {
                bail!("unknown hash {part:?}: use xxh64, md5 or sha1 (comma-separated)");
            };
            if !algos.contains(&algo) {
                algos.push(algo);
            }
        }
        Ok(algos)
    }
}

/// One streaming hasher per requested algorithm, fed from a single read.
pub struct MultiHasher {
    xxh64: Xxh64,
    md5: Option<Md5>,
    sha1: Option<Sha1>,
}

impl MultiHasher {
    pub fn new(algos: &[Algo]) -> Self {
        Self {
            xxh64: Xxh64::new(0),
            md5: algos.contains(&Algo::Md5).then(Md5::new),
            sha1: algos.contains(&Algo::Sha1).then(Sha1::new),
        }
    }

    pub fn update(&mut self, data: &[u8]) {
        self.xxh64.update(data);
        if let Some(h) = self.md5.as_mut() {
            h.update(data);
        }
        if let Some(h) = self.sha1.as_mut() {
            h.update(data);
        }
    }

    /// Lowercase hex digests, xxh64 first.
    pub fn finish(self) -> Vec<(Algo, String)> {
        let mut out = vec![(Algo::Xxh64, format!("{:016x}", self.xxh64.digest()))];
        if let Some(h) = self.md5 {
            out.push((Algo::Md5, hex(&h.finalize())));
        }
        if let Some(h) = self.sha1 {
            out.push((Algo::Sha1, hex(&h.finalize())));
        }
        out
    }
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Read size for every copy and hash pass. Large enough that USB and NVMe
/// reads run at full speed, small enough that up to `MAX_IN_FLIGHT` chunks
/// per destination stay well under 100 MB of memory.
pub const CHUNK: usize = 8 << 20;

/// Hash a whole file with the given algorithms (one read).
pub fn hash_file(
    path: &std::path::Path,
    algos: &[Algo],
) -> std::io::Result<(u64, Vec<(Algo, String)>)> {
    use std::io::Read;
    let mut file = std::fs::File::open(path)?;
    let mut hasher = MultiHasher::new(algos);
    let mut buf = vec![0u8; CHUNK];
    let mut total = 0u64;
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        total += n as u64;
    }
    Ok((total, hasher.finish()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xxh64_of_known_input() {
        let mut h = MultiHasher::new(&[Algo::Xxh64, Algo::Md5, Algo::Sha1]);
        h.update(b"abc");
        let out = h.finish();
        assert_eq!(out[0], (Algo::Xxh64, "44bc2cf5ad770999".to_string()));
        assert_eq!(
            out[1],
            (Algo::Md5, "900150983cd24fb0d6963f7d28e17f72".to_string())
        );
        assert_eq!(
            out[2],
            (
                Algo::Sha1,
                "a9993e364706816aba3e25717850c26c9cd0d89d".to_string()
            )
        );
    }

    #[test]
    fn parse_list_always_starts_with_xxh64() {
        assert_eq!(
            Algo::parse_list("md5").unwrap(),
            vec![Algo::Xxh64, Algo::Md5]
        );
        assert_eq!(
            Algo::parse_list("sha1,xxh64").unwrap(),
            vec![Algo::Xxh64, Algo::Sha1]
        );
        assert!(Algo::parse_list("crc32").is_err());
    }
}
