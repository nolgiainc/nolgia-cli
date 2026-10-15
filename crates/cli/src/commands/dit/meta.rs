//! Camera metadata for the inventory, the rename and the report, read
//! natively so `nolgia dit` works on a bare laptop:
//!
//! - Sony XAVC sidecars (`C0001M01.XML`, NonRealTimeMeta): model, lens,
//!   capture/format frame rate, resolution, codec, timecode, gamma
//!   (S-Log3 ...), gamut, duration, creation time with its UTC offset.
//! - EXIF in JPEG and in TIFF-based raws (CR2, NEF, ARW, DNG, ORF, RW2...).
//! - Canon CR3 (ISO base media): the CMT1/CMT2 TIFF blocks in Canon's
//!   `uuid` box.
//!
//! When `ffprobe` or `exiftool` is on PATH, they fill in what the native
//! readers cannot (duration of a plain MP4/MOV, HEIC dates). Nothing here
//! ever writes to a file.

use std::{
    fs,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    process::Command,
};

use chrono::{DateTime, FixedOffset, NaiveDateTime};
use quick_xml::{Reader, events::Event};
use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Video,
    Photo,
    Audio,
    Other,
}

const VIDEO_EXT: &[&str] = &[
    "mp4", "mov", "mxf", "braw", "r3d", "crm", "avi", "mts", "m2ts", "mkv", "insv", "360", "lrv",
];
const PHOTO_EXT: &[&str] = &[
    "cr3", "cr2", "crw", "nef", "nrw", "arw", "srf", "sr2", "dng", "raf", "rw2", "orf", "pef",
    "srw", "x3f", "iiq", "3fr", "jpg", "jpeg", "heic", "heif", "hif", "tif", "tiff", "png",
];
const AUDIO_EXT: &[&str] = &["wav", "bwf", "mp3", "aac", "m4a", "aif", "aiff", "flac"];

pub fn kind_of(path: &Path) -> Kind {
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    if VIDEO_EXT.contains(&ext.as_str()) && ext != "lrv" {
        Kind::Video
    } else if PHOTO_EXT.contains(&ext.as_str()) {
        Kind::Photo
    } else if AUDIO_EXT.contains(&ext.as_str()) {
        Kind::Audio
    } else {
        Kind::Other
    }
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Meta {
    pub make: Option<String>,
    pub model: Option<String>,
    pub lens: Option<String>,
    /// Capture time as the camera recorded it, with its UTC offset when the
    /// camera wrote one. Never corrected here; the clock offset is applied by
    /// the caller so the original stays visible.
    pub captured: Option<String>,
    pub duration_s: Option<f64>,
    pub capture_fps: Option<String>,
    pub format_fps: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub codec: Option<String>,
    pub start_timecode: Option<String>,
    pub gamma: Option<String>,
    pub gamut: Option<String>,
    pub iso: Option<u32>,
    pub audio_channels: Option<u32>,
    /// Which reader produced this: `sony-xml`, `exif`, `cr3`, `ffprobe`, `exiftool`, `file-time`.
    pub source: Option<String>,
}

impl Meta {
    /// The capture time as a fixed-offset datetime (naive local when the
    /// camera wrote no offset).
    pub fn captured_dt(&self) -> Option<NaiveDateTime> {
        let s = self.captured.as_deref()?;
        if let Ok(dt) = DateTime::<FixedOffset>::parse_from_rfc3339(s) {
            return Some(dt.naive_local());
        }
        for fmt in [
            "%Y:%m:%d %H:%M:%S",
            "%Y-%m-%dT%H:%M:%S",
            "%Y-%m-%d %H:%M:%S",
        ] {
            if let Ok(dt) = NaiveDateTime::parse_from_str(s.get(..19).unwrap_or(s), fmt) {
                return Some(dt);
            }
        }
        None
    }

    /// The camera's name for grouping: model, else make, else "unknown".
    pub fn camera(&self) -> String {
        self.model
            .clone()
            .or_else(|| self.make.clone())
            .unwrap_or_else(|| "unknown".to_string())
    }
}

/// Sony's sidecar for a clip: `C0001.MP4` -> `C0001M01.XML` (on the card),
/// or `<same stem>.XML` (after a rename that kept the pair together).
pub fn sony_sidecar(clip: &Path) -> Option<PathBuf> {
    let stem = clip.file_stem()?.to_string_lossy().to_string();
    let dir = clip.parent()?;
    for name in [
        format!("{stem}M01.XML"),
        format!("{stem}M01.xml"),
        format!("{stem}.XML"),
        format!("{stem}.xml"),
    ] {
        let p = dir.join(name);
        if p.is_file() {
            return Some(p);
        }
    }
    None
}

/// Read everything we can about one media file. Never fails: an unreadable
/// file yields only its file time, marked as such.
pub fn read(path: &Path, tools: &Tools) -> Meta {
    let mut meta = Meta::default();
    match kind_of(path) {
        Kind::Video => {
            if let Some(xml) = sony_sidecar(path)
                && let Ok(text) = fs::read_to_string(&xml)
            {
                meta = parse_sony_xml(&text);
            }
            if (meta.duration_s.is_none() || meta.captured.is_none())
                && let Some(probe) = tools.ffprobe.as_ref().and_then(|ff| ffprobe(ff, path))
            {
                fill(&mut meta, probe);
            }
        }
        Kind::Photo => {
            let ext = path
                .extension()
                .map(|e| e.to_string_lossy().to_ascii_lowercase())
                .unwrap_or_default();
            let native = if ext == "cr3" {
                read_cr3(path)
            } else {
                read_exif_file(path)
            };
            if let Some(m) = native {
                meta = m;
            }
            if meta.captured.is_none()
                && let Some(m) = tools.exiftool.as_ref().and_then(|et| exiftool(et, path))
            {
                fill(&mut meta, m);
            }
        }
        Kind::Audio | Kind::Other => {}
    }
    if meta.captured.is_none()
        && let Some(t) = fs::metadata(path).ok().and_then(|m| m.modified().ok())
    {
        let dt: DateTime<chrono::Local> = t.into();
        meta.captured = Some(dt.to_rfc3339());
        if meta.source.is_none() {
            meta.source = Some("file-time".to_string());
        }
    }
    meta
}

fn fill(meta: &mut Meta, other: Meta) {
    macro_rules! take {
        ($($f:ident),*) => { $( if meta.$f.is_none() { meta.$f = other.$f.clone(); } )* };
    }
    take!(
        make,
        model,
        lens,
        captured,
        duration_s,
        capture_fps,
        format_fps,
        width,
        height,
        codec,
        start_timecode,
        gamma,
        gamut,
        iso,
        audio_channels
    );
    if meta.source.is_none() {
        meta.source = other.source;
    }
}

// --------------------------------------------------------- Sony XML --

/// Sony writes LTC as BCD bytes `FFSSMMHH` with flag bits (drop-frame,
/// colour frame) set in the high bits; mask them off.
fn sony_ltc(value: &str) -> Option<String> {
    let v = format!("{value:0>8}");
    let byte = |i: usize| u8::from_str_radix(v.get(i..i + 2)?, 16).ok();
    let (ff, ss, mm, hh) = (
        byte(0)? & 0x3f,
        byte(2)? & 0x7f,
        byte(4)? & 0x7f,
        byte(6)? & 0x3f,
    );
    Some(format!("{hh:02x}:{mm:02x}:{ss:02x}:{ff:02x}"))
}

pub fn parse_sony_xml(text: &str) -> Meta {
    let mut m = Meta {
        source: Some("sony-xml".to_string()),
        ..Default::default()
    };
    let mut reader = Reader::from_str(text);
    let mut frames: Option<f64> = None;
    let mut ltc_first: Option<String> = None;
    loop {
        let ev = reader.read_event();
        let e = match &ev {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) => e.clone(),
            Ok(Event::Eof) | Err(_) => break,
            _ => continue,
        };
        let name = String::from_utf8_lossy(e.local_name().as_ref()).to_string();
        let attr = |key: &str| -> Option<String> {
            e.attributes()
                .flatten()
                .find(|a| a.key.local_name().as_ref() == key.as_bytes())
                .map(|a| String::from_utf8_lossy(&a.value).to_string())
        };
        match name.as_str() {
            "Duration" => frames = attr("value").and_then(|v| v.parse().ok()),
            "CreationDate" => m.captured = attr("value"),
            "LtcChange" if ltc_first.is_none() => ltc_first = attr("value"),
            "VideoFrame" => {
                m.codec = attr("videoCodec");
                m.capture_fps = attr("captureFps");
                m.format_fps = attr("formatFps");
            }
            "VideoLayout" => {
                m.width = attr("pixel").and_then(|v| v.parse().ok());
                m.height = attr("numOfVerticalLine").and_then(|v| v.parse().ok());
            }
            "AudioFormat" => m.audio_channels = attr("numOfChannel").and_then(|v| v.parse().ok()),
            "Device" => {
                m.make = attr("manufacturer");
                m.model = attr("modelName");
            }
            "Lens" => m.lens = attr("modelName"),
            "Item" => match attr("name").as_deref() {
                Some("CaptureGammaEquation") => m.gamma = attr("value"),
                Some("CaptureColorPrimaries") => m.gamut = attr("value"),
                _ => {}
            },
            _ => {}
        }
    }
    m.start_timecode = ltc_first.as_deref().and_then(sony_ltc);
    let fps = m
        .format_fps
        .as_deref()
        .and_then(|f| f.trim_end_matches(['p', 'i']).parse::<f64>().ok());
    if let (Some(n), Some(fps)) = (frames, fps)
        && fps > 0.0
    {
        m.duration_s = Some((n / fps * 100.0).round() / 100.0);
    }
    m
}

// ------------------------------------------------------------- TIFF --

/// The handful of tags we report, from any TIFF structure (an EXIF APP1
/// block, a TIFF raw, or one of Canon's CMT blocks). Sub-IFDs are followed
/// through the EXIF pointer (0x8769).
fn parse_tiff(data: &[u8], m: &mut Meta) -> bool {
    if data.len() < 8 {
        return false;
    }
    let le = match &data[0..2] {
        b"II" => true,
        b"MM" => false,
        _ => return false,
    };
    let u16_at = |o: usize| -> Option<u16> {
        let b = data.get(o..o + 2)?;
        Some(if le {
            u16::from_le_bytes([b[0], b[1]])
        } else {
            u16::from_be_bytes([b[0], b[1]])
        })
    };
    let u32_at = |o: usize| -> Option<u32> {
        let b = data.get(o..o + 4)?;
        let a = [b[0], b[1], b[2], b[3]];
        Some(if le {
            u32::from_le_bytes(a)
        } else {
            u32::from_be_bytes(a)
        })
    };
    let Some(first) = u32_at(4) else { return false };
    let mut found = false;
    let mut queue = vec![first as usize];
    let mut seen = 0;
    while let Some(ifd) = queue.pop() {
        seen += 1;
        if seen > 8 || ifd == 0 {
            continue;
        }
        let Some(count) = u16_at(ifd) else { continue };
        for i in 0..count as usize {
            let e = ifd + 2 + i * 12;
            let (Some(tag), Some(typ), Some(n), Some(val)) =
                (u16_at(e), u16_at(e + 2), u32_at(e + 4), u32_at(e + 8))
            else {
                break;
            };
            let unit = match typ {
                1 | 2 | 7 => 1,
                3 => 2,
                4 | 9 => 4,
                5 | 10 => 8,
                _ => 1,
            };
            let inline = (unit * n as usize) <= 4;
            let off = if inline { e + 8 } else { val as usize };
            let ascii = || -> Option<String> {
                let b = data.get(off..off + n as usize)?;
                let s = String::from_utf8_lossy(b)
                    .trim_end_matches('\0')
                    .trim()
                    .to_string();
                (!s.is_empty()).then_some(s)
            };
            let short_or_long = || -> Option<u32> {
                match typ {
                    3 => u16_at(off).map(u32::from),
                    4 => u32_at(off),
                    _ => None,
                }
            };
            match tag {
                0x010f => m.make = ascii().or(m.make.take()),
                0x0110 => m.model = ascii().or(m.model.take()),
                0x9003 => {
                    if let Some(s) = ascii() {
                        m.captured = Some(s);
                        found = true;
                    }
                }
                0x0132 if m.captured.is_none() => m.captured = ascii(),
                0x9011 => {
                    // OffsetTimeOriginal: append to the capture time when we have one.
                    if let (Some(off), Some(c)) = (ascii(), m.captured.as_ref())
                        && c.len() == 19
                        && let Ok(dt) = NaiveDateTime::parse_from_str(c, "%Y:%m:%d %H:%M:%S")
                    {
                        m.captured = Some(format!("{}{}", dt.format("%Y-%m-%dT%H:%M:%S"), off));
                    }
                }
                0xa434 => m.lens = ascii(),
                0x8827 => m.iso = short_or_long(),
                0xa002 => m.width = short_or_long().or(m.width),
                0xa003 => m.height = short_or_long().or(m.height),
                0x8769 => queue.push(val as usize),
                _ => {}
            }
        }
    }
    // OffsetTimeOriginal can come before DateTimeOriginal in tag order; a
    // second pass is not worth it, the naive time is still correct local time.
    found || m.model.is_some()
}

fn read_exif_file(path: &Path) -> Option<Meta> {
    let mut f = fs::File::open(path).ok()?;
    let mut head = vec![0u8; 256 * 1024];
    let n = f.read(&mut head).ok()?;
    head.truncate(n);
    let mut m = Meta {
        source: Some("exif".to_string()),
        ..Default::default()
    };
    if head.starts_with(b"II") || head.starts_with(b"MM") {
        // TIFF-based raw: read enough of the file for the IFDs it points to.
        let mut all = head;
        if n == 256 * 1024 {
            let mut more = vec![0u8; 4 * 1024 * 1024];
            let k = f.read(&mut more).ok()?;
            all.extend_from_slice(&more[..k]);
        }
        return parse_tiff(&all, &mut m).then_some(m);
    }
    // JPEG: find the APP1 "Exif\0\0" segment.
    if head.starts_with(&[0xff, 0xd8]) {
        let mut i = 2;
        while i + 4 < head.len() && head[i] == 0xff {
            let marker = head[i + 1];
            let len = u16::from_be_bytes([head[i + 2], head[i + 3]]) as usize;
            if marker == 0xe1 && head.get(i + 4..i + 10) == Some(b"Exif\0\0") {
                let tiff = head.get(i + 10..(i + 2 + len).min(head.len()))?;
                return parse_tiff(tiff, &mut m).then_some(m);
            }
            i += 2 + len;
        }
    }
    None
}

// -------------------------------------------------------------- CR3 --

const CANON_UUID: [u8; 16] = [
    0x85, 0xc0, 0xb6, 0x87, 0x82, 0x0f, 0x11, 0xe0, 0x81, 0x11, 0xf4, 0xce, 0x46, 0x2b, 0x6a, 0x48,
];

/// Canon CR3: `moov` holds a `uuid` box (Canon's UUID) whose children
/// CMT1 (IFD0: make, model) and CMT2 (EXIF: capture time, ISO, lens) are
/// complete little TIFF files.
pub fn read_cr3(path: &Path) -> Option<Meta> {
    let mut f = fs::File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    let moov = find_box(&mut f, 0, len, b"moov")?;
    let (canon_start, canon_end) = find_uuid(&mut f, moov.0, moov.1, &CANON_UUID)?;
    let mut m = Meta {
        source: Some("cr3".to_string()),
        ..Default::default()
    };
    let mut any = false;
    for name in [b"CMT1", b"CMT2"] {
        if let Some((s, e)) = find_box(&mut f, canon_start, canon_end, name) {
            let mut buf = vec![0u8; (e - s).min(1 << 20) as usize];
            f.seek(SeekFrom::Start(s)).ok()?;
            f.read_exact(&mut buf).ok()?;
            any |= parse_tiff(&buf, &mut m);
        }
    }
    any.then_some(m)
}

/// Find a child box by type inside [start, end); returns its payload range.
fn find_box(f: &mut fs::File, start: u64, end: u64, kind: &[u8; 4]) -> Option<(u64, u64)> {
    let mut pos = start;
    while pos + 8 <= end {
        f.seek(SeekFrom::Start(pos)).ok()?;
        let mut hdr = [0u8; 8];
        f.read_exact(&mut hdr).ok()?;
        let mut size = u32::from_be_bytes([hdr[0], hdr[1], hdr[2], hdr[3]]) as u64;
        let mut header = 8u64;
        if size == 1 {
            let mut ext = [0u8; 8];
            f.read_exact(&mut ext).ok()?;
            size = u64::from_be_bytes(ext);
            header = 16;
        } else if size == 0 {
            size = end - pos;
        }
        if size < header || pos + size > end {
            return None;
        }
        if &hdr[4..8] == kind {
            return Some((pos + header, pos + size));
        }
        pos += size;
    }
    None
}

fn find_uuid(f: &mut fs::File, start: u64, end: u64, uuid: &[u8; 16]) -> Option<(u64, u64)> {
    let mut pos = start;
    while pos + 8 <= end {
        let (s, e) = find_box(f, pos, end, b"uuid")?;
        let mut id = [0u8; 16];
        f.seek(SeekFrom::Start(s)).ok()?;
        f.read_exact(&mut id).ok()?;
        if &id == uuid {
            return Some((s + 16, e));
        }
        pos = e;
    }
    None
}

// ------------------------------------------------ optional helpers --

#[derive(Clone, Debug, Default)]
pub struct Tools {
    pub ffprobe: Option<PathBuf>,
    pub exiftool: Option<PathBuf>,
}

impl Tools {
    pub fn detect() -> Self {
        Self {
            ffprobe: which("ffprobe"),
            exiftool: which("exiftool"),
        }
    }
}

fn which(name: &str) -> Option<PathBuf> {
    let exts: &[&str] = if cfg!(windows) {
        &[".exe", ".cmd", ".bat"]
    } else {
        &[""]
    };
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        for ext in exts {
            let p = dir.join(format!("{name}{ext}"));
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

fn ffprobe(ff: &Path, path: &Path) -> Option<Meta> {
    let out = Command::new(ff)
        .args([
            "-v",
            "error",
            "-print_format",
            "json",
            "-show_format",
            "-show_streams",
        ])
        .arg(path)
        .output()
        .ok()?;
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
    let mut m = Meta {
        source: Some("ffprobe".to_string()),
        ..Default::default()
    };
    m.duration_s = v["format"]["duration"]
        .as_str()
        .and_then(|d| d.parse().ok());
    m.captured = v["format"]["tags"]["creation_time"]
        .as_str()
        .map(str::to_string);
    if let Some(streams) = v["streams"].as_array() {
        for s in streams {
            if s["codec_type"] == "video" && m.width.is_none() {
                m.width = s["width"].as_u64().map(|x| x as u32);
                m.height = s["height"].as_u64().map(|x| x as u32);
                m.codec = s["codec_name"].as_str().map(str::to_string);
                m.format_fps = s["avg_frame_rate"].as_str().map(str::to_string);
            }
            if s["codec_type"] == "audio" && m.audio_channels.is_none() {
                m.audio_channels = s["channels"].as_u64().map(|x| x as u32);
            }
        }
    }
    Some(m)
}

fn exiftool(et: &Path, path: &Path) -> Option<Meta> {
    let out = Command::new(et)
        .args([
            "-j",
            "-n",
            "-Make",
            "-Model",
            "-LensModel",
            "-DateTimeOriginal",
            "-OffsetTimeOriginal",
            "-ISO",
        ])
        .arg(path)
        .output()
        .ok()?;
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
    let o = v.get(0)?;
    let s = |k: &str| o[k].as_str().map(str::to_string);
    let mut m = Meta {
        source: Some("exiftool".to_string()),
        ..Default::default()
    };
    m.make = s("Make");
    m.model = s("Model");
    m.lens = s("LensModel");
    m.captured = s("DateTimeOriginal");
    m.iso = o["ISO"].as_u64().map(|x| x as u32);
    Some(m)
}

#[cfg(test)]
pub mod tests {
    use super::*;

    pub const SONY_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<NonRealTimeMeta xmlns="urn:schemas-professionalDisc:nonRealTimeMeta:ver.2.20">
	<Duration value="255"/>
	<LtcChangeTable tcFps="30" halfStep="false">
		<LtcChange frameCount="0" value="57013716" status="increment"/>
		<LtcChange frameCount="254" value="41103716" status="end"/>
	</LtcChangeTable>
	<CreationDate value="2026-10-13T01:01:25-08:00"/>
	<VideoFormat>
		<VideoFrame videoCodec="AVC_3840_2160_HP@L51" captureFps="29.97p" formatFps="29.97p"/>
		<VideoLayout pixel="3840" numOfVerticalLine="2160" aspectRatio="16:9"/>
	</VideoFormat>
	<AudioFormat numOfChannel="2"/>
	<Device manufacturer="Sony" modelName="ILME-FX3A" serialNo="4294967295"/>
	<Lens modelName="24-70mm F2.8 DG DN II | Art 024"/>
	<AcquisitionRecord>
		<Group name="CameraUnitMetadataSet">
			<Item name="CaptureGammaEquation" value="s-log3-cine"/>
			<Item name="CaptureColorPrimaries" value="s-gamut3-cine"/>
		</Group>
	</AcquisitionRecord>
</NonRealTimeMeta>"#;

    #[test]
    fn sony_sidecar_fields() {
        let m = parse_sony_xml(SONY_XML);
        assert_eq!(m.model.as_deref(), Some("ILME-FX3A"));
        assert_eq!(m.gamma.as_deref(), Some("s-log3-cine"));
        assert_eq!(m.width, Some(3840));
        assert_eq!(m.start_timecode.as_deref(), Some("16:37:01:17"));
        assert_eq!(m.duration_s, Some(8.51));
        assert_eq!(
            m.captured_dt().unwrap().format("%Y%m%d %H%M").to_string(),
            "20261013 0101"
        );
    }

    /// A minimal little-endian TIFF with Make, Model and an EXIF IFD holding
    /// DateTimeOriginal and ISO. Used for the JPEG and CR3 fixtures too.
    pub fn tiff_block(make: &str, model: &str, when: &str, iso: u16) -> Vec<u8> {
        let mut t = b"II*\0\x08\0\0\0".to_vec();
        // IFD0 at 8: 3 entries (Make, Model, ExifIFD), next=0.
        let ifd0_len = 2 + 3 * 12 + 4;
        let strings_at = 8 + ifd0_len;
        let make_b = format!("{make}\0").into_bytes();
        let model_b = format!("{model}\0").into_bytes();
        let exif_at = strings_at + make_b.len() + model_b.len();
        let mut ifd = vec![];
        ifd.extend(3u16.to_le_bytes());
        let entry = |tag: u16, typ: u16, n: u32, val: u32| {
            let mut e = vec![];
            e.extend(tag.to_le_bytes());
            e.extend(typ.to_le_bytes());
            e.extend(n.to_le_bytes());
            e.extend(val.to_le_bytes());
            e
        };
        ifd.extend(entry(0x010f, 2, make_b.len() as u32, strings_at as u32));
        ifd.extend(entry(
            0x0110,
            2,
            model_b.len() as u32,
            (strings_at + make_b.len()) as u32,
        ));
        ifd.extend(entry(0x8769, 4, 1, exif_at as u32));
        ifd.extend(0u32.to_le_bytes());
        t.extend(ifd);
        t.extend(make_b);
        t.extend(model_b);
        // EXIF IFD: DateTimeOriginal (ascii, 20 bytes) + ISO (short, inline).
        let when_b = format!("{when}\0").into_bytes();
        let exif_len = 2 + 2 * 12 + 4;
        let when_at = exif_at + exif_len;
        let mut ex = vec![];
        ex.extend(2u16.to_le_bytes());
        ex.extend(entry(0x9003, 2, when_b.len() as u32, when_at as u32));
        let mut iso_e = entry(0x8827, 3, 1, 0);
        iso_e[8..10].copy_from_slice(&iso.to_le_bytes());
        ex.extend(iso_e);
        ex.extend(0u32.to_le_bytes());
        t.extend(ex);
        t.extend(when_b);
        t
    }

    pub fn jpeg_with_exif(make: &str, model: &str, when: &str) -> Vec<u8> {
        let tiff = tiff_block(make, model, when, 400);
        let mut j = vec![0xff, 0xd8, 0xff, 0xe1];
        j.extend(((tiff.len() + 8) as u16).to_be_bytes());
        j.extend(b"Exif\0\0");
        j.extend(tiff);
        j.extend([0xff, 0xd9]);
        j
    }

    fn bx(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut b = ((payload.len() + 8) as u32).to_be_bytes().to_vec();
        b.extend(kind);
        b.extend(payload);
        b
    }

    pub fn cr3_with_exif(model: &str, when: &str) -> Vec<u8> {
        let cmt1 = tiff_block("Canon", model, when, 800);
        let mut uuid_payload = CANON_UUID.to_vec();
        uuid_payload.extend(bx(b"CMT1", &cmt1));
        let moov = bx(b"moov", &bx(b"uuid", &uuid_payload));
        let mut f = bx(b"ftyp", b"crx \0\0\0\x01crx isom");
        f.extend(moov);
        f.extend(bx(b"mdat", &[0u8; 64]));
        f
    }

    #[test]
    fn jpeg_and_cr3_exif() {
        let dir = tempfile::tempdir().unwrap();
        let j = dir.path().join("a.JPG");
        fs::write(
            &j,
            jpeg_with_exif("Canon", "Canon EOS R6m2", "2026:10:12 13:27:14"),
        )
        .unwrap();
        let m = read_exif_file(&j).unwrap();
        assert_eq!(m.model.as_deref(), Some("Canon EOS R6m2"));
        assert_eq!(m.iso, Some(400));
        let c = dir.path().join("GK1A0001.CR3");
        fs::write(&c, cr3_with_exif("Canon EOS R6m2", "2026:10:12 13:27:14")).unwrap();
        let m = read_cr3(&c).unwrap();
        assert_eq!(m.captured_dt().unwrap().format("%H%M").to_string(), "1327");
        assert_eq!(m.iso, Some(800));
    }
}
