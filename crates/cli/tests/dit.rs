//! `nolgia dit` end to end on synthetic Sony and Canon cards. Offline: no
//! API, no login, temp folders only.

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use assert_cmd::Command;
use serde_json::Value;

const SONY_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<NonRealTimeMeta xmlns="urn:schemas-professionalDisc:nonRealTimeMeta:ver.2.20">
	<Duration value="255"/>
	<LtcChangeTable tcFps="30" halfStep="false">
		<LtcChange frameCount="0" value="57013716" status="increment"/>
	</LtcChangeTable>
	<CreationDate value="WHEN"/>
	<VideoFormat>
		<VideoFrame videoCodec="AVC_3840_2160_HP@L51" captureFps="29.97p" formatFps="29.97p"/>
		<VideoLayout pixel="3840" numOfVerticalLine="2160" aspectRatio="16:9"/>
	</VideoFormat>
	<Device manufacturer="Sony" modelName="ILME-FX3A" serialNo="1"/>
	<AcquisitionRecord><Group name="CameraUnitMetadataSet">
		<Item name="CaptureGammaEquation" value="s-log3-cine"/>
	</Group></AcquisitionRecord>
</NonRealTimeMeta>"#;

fn nolgia() -> Command {
    let mut c = Command::cargo_bin("nolgia").unwrap();
    c.env_remove("NOLGIA_DIT_TEST_STOP_AFTER_FILES");
    c
}

/// Bytes that differ per file so a wrong copy cannot pass by accident.
fn noise(seed: u64, len: usize) -> Vec<u8> {
    let mut x = seed.wrapping_mul(0x9e3779b97f4a7c15) | 1;
    (0..len)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x & 0xff) as u8
        })
        .collect()
}

fn tiff_block(model: &str, when: &str) -> Vec<u8> {
    // II, IFD0 at 8 with Model + ExifIFD pointer; EXIF IFD with DateTimeOriginal.
    let model_b = format!("{model}\0").into_bytes();
    let when_b = format!("{when}\0").into_bytes();
    let ifd0 = 8usize;
    let ifd0_len = 2 + 2 * 12 + 4;
    let model_at = ifd0 + ifd0_len;
    let exif_at = model_at + model_b.len();
    let exif_len = 2 + 12 + 4;
    let when_at = exif_at + exif_len;
    let entry = |tag: u16, typ: u16, n: u32, val: u32| {
        let mut e = tag.to_le_bytes().to_vec();
        e.extend(typ.to_le_bytes());
        e.extend(n.to_le_bytes());
        e.extend(val.to_le_bytes());
        e
    };
    let mut t = b"II*\0\x08\0\0\0".to_vec();
    t.extend(2u16.to_le_bytes());
    t.extend(entry(0x0110, 2, model_b.len() as u32, model_at as u32));
    t.extend(entry(0x8769, 4, 1, exif_at as u32));
    t.extend(0u32.to_le_bytes());
    t.extend(model_b);
    t.extend(1u16.to_le_bytes());
    t.extend(entry(0x9003, 2, when_b.len() as u32, when_at as u32));
    t.extend(0u32.to_le_bytes());
    t.extend(when_b);
    t
}

fn bx(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
    let mut b = ((payload.len() + 8) as u32).to_be_bytes().to_vec();
    b.extend(kind);
    b.extend(payload);
    b
}

fn cr3(model: &str, when: &str, seed: u64) -> Vec<u8> {
    let uuid = [
        0x85, 0xc0, 0xb6, 0x87, 0x82, 0x0f, 0x11, 0xe0, 0x81, 0x11, 0xf4, 0xce, 0x46, 0x2b, 0x6a,
        0x48,
    ];
    let mut up = uuid.to_vec();
    up.extend(bx(b"CMT1", &tiff_block(model, when)));
    let mut f = bx(b"ftyp", b"crx \0\0\0\x01crx isom");
    f.extend(bx(b"moov", &bx(b"uuid", &up)));
    f.extend(bx(b"mdat", &noise(seed, 200_000)));
    f
}

/// A Sony card: three clips with sidecars, card files, an empty folder,
/// and OS clutter that must not be copied.
fn sony_card(root: &Path) {
    let clip = root.join("PRIVATE/M4ROOT/CLIP");
    fs::create_dir_all(&clip).unwrap();
    fs::create_dir_all(root.join("PRIVATE/M4ROOT/SUB")).unwrap();
    fs::create_dir_all(root.join("PRIVATE/M4ROOT/THMBNL")).unwrap();
    for (i, when) in [
        "2026-10-13T01:01:25-08:00",
        "2026-10-13T04:30:00-08:00",
        "2026-10-13T08:17:51-08:00",
    ]
    .iter()
    .enumerate()
    {
        let n = i + 1;
        fs::write(
            clip.join(format!("C000{n}.MP4")),
            noise(n as u64, 3_000_000 + n * 1000),
        )
        .unwrap();
        fs::write(
            clip.join(format!("C000{n}M01.XML")),
            SONY_XML.replace("WHEN", when),
        )
        .unwrap();
        fs::write(
            root.join(format!("PRIVATE/M4ROOT/THMBNL/C000{n}T01.JPG")),
            noise(50 + n as u64, 900),
        )
        .unwrap();
    }
    fs::write(root.join("PRIVATE/M4ROOT/MEDIAPRO.XML"), "<MediaProfile/>").unwrap();
    fs::create_dir_all(root.join("System Volume Information")).unwrap();
    fs::write(
        root.join("System Volume Information/IndexerVolumeGuid"),
        "x",
    )
    .unwrap();
}

fn canon_card(root: &Path) {
    let d = root.join("DCIM/100EOSR6");
    fs::create_dir_all(&d).unwrap();
    for (i, when) in [
        "2026:10:12 13:27:14",
        "2026:10:12 17:00:00",
        "2026:10:12 20:13:55",
    ]
    .iter()
    .enumerate()
    {
        fs::write(
            d.join(format!("GK1A000{}.CR3", i + 1)),
            cr3("Canon EOS R6m2", when, 100 + i as u64),
        )
        .unwrap();
    }
    fs::create_dir_all(root.join("DCIM/CANONMSC")).unwrap();
    fs::write(root.join("DCIM/CANONMSC/M0001.CTG"), noise(7, 512)).unwrap();
}

/// Every file under a folder with its bytes (skipping our bookkeeping).
fn tree(root: &Path) -> BTreeMap<String, Vec<u8>> {
    fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
        for e in fs::read_dir(dir).unwrap().flatten() {
            let p = e.path();
            let rel = p
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            if rel.starts_with("ascmhl") || rel.starts_with("_DIT") {
                continue;
            }
            if p.is_dir() {
                walk(root, &p, out);
            } else {
                out.insert(rel, fs::read(&p).unwrap());
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(root, root, &mut out);
    out
}

fn json(out: &std::process::Output) -> Value {
    serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
        panic!(
            "not JSON ({e}): {}\nstderr: {}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        )
    })
}

struct Fixture {
    _tmp: tempfile::TempDir,
    sony: PathBuf,
    canon: PathBuf,
    d1: PathBuf,
    d2: PathBuf,
}

fn fixture() -> Fixture {
    let tmp = tempfile::tempdir().unwrap();
    let sony = tmp.path().join("cards/A001");
    let canon = tmp.path().join("cards/B001");
    sony_card(&sony);
    canon_card(&canon);
    let d1 = tmp.path().join("drive1");
    let d2 = tmp.path().join("drive2");
    fs::create_dir_all(&d1).unwrap();
    fs::create_dir_all(&d2).unwrap();
    Fixture {
        _tmp: tmp,
        sony,
        canon,
        d1,
        d2,
    }
}

fn offload(f: &Fixture) -> Command {
    let mut c = nolgia();
    c.args(["--json", "dit", "offload"])
        .arg(&f.sony)
        .arg(&f.canon)
        .arg("--to")
        .arg(&f.d1)
        .arg("--to")
        .arg(&f.d2)
        .args(["--shoot", "TEST", "--hash", "xxh64,md5"]);
    c
}

#[test]
fn offload_two_cards_to_two_drives_then_verify() {
    let f = fixture();
    let out = offload(&f).output().unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v = json(&out);
    assert_eq!(v["status"], "complete");
    // The Sony card's files, minus the OS clutter that is never copied.
    let sony_files = tree(&f.sony)
        .into_iter()
        .filter(|(k, _)| !k.starts_with("System Volume"))
        .collect::<BTreeMap<_, _>>();
    for d in [&f.d1, &f.d2] {
        assert_eq!(
            tree(&d.join("TEST/A001")),
            sony_files,
            "Sony copy at {}",
            d.display()
        );
        assert_eq!(
            tree(&d.join("TEST/B001")),
            tree(&f.canon),
            "Canon copy at {}",
            d.display()
        );
        assert!(
            d.join("TEST/A001/PRIVATE/M4ROOT/SUB").is_dir(),
            "empty card folders are recreated"
        );
        assert!(
            !d.join("TEST/A001/System Volume Information").exists(),
            "OS clutter is not copied"
        );
        assert!(d.join("TEST/A001/ascmhl/ascmhl_chain.xml").is_file());
        assert!(d.join("TEST/_DIT/A001.manifest.tsv").is_file());
        let tsv = fs::read_to_string(d.join("TEST/_DIT/B001.manifest.tsv")).unwrap();
        assert!(
            tsv.contains("xxh64\tmd5\tbytes\tmtime\trelative_path"),
            "{tsv}"
        );
    }
    assert!(v["report"].as_str().is_some_and(|r| Path::new(r).is_file()));
    // A report in each destination's shoot folder.
    let reports: Vec<&str> = v["reports"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r.as_str().unwrap())
        .collect();
    assert_eq!(reports.len(), 2, "{v}");
    for (r, d) in reports.iter().zip([&f.d1, &f.d2]) {
        assert!(Path::new(r).starts_with(d.join("TEST/_DIT")), "{r}");
        assert!(Path::new(r).is_file());
    }
    for card in ["A001", "B001"] {
        let out = nolgia()
            .args(["--json", "dit", "verify"])
            .arg(f.d2.join("TEST").join(card))
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stdout)
        );
        assert_eq!(json(&out)["status"], "ok");
    }
}

#[test]
fn verify_catches_a_flipped_bit() {
    let f = fixture();
    assert!(offload(&f).output().unwrap().status.success());
    let victim = f.d1.join("TEST/A001/PRIVATE/M4ROOT/CLIP/C0002.MP4");
    let mut bytes = fs::read(&victim).unwrap();
    bytes[1_234_567] ^= 0x01;
    fs::write(&victim, bytes).unwrap();
    let out = nolgia()
        .args(["--json", "dit", "verify"])
        .arg(f.d1.join("TEST/A001"))
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(68));
    let v = json(&out);
    assert_eq!(v["status"], "failed");
    assert_eq!(v["mismatched"][0]["path"], "PRIVATE/M4ROOT/CLIP/C0002.MP4");
    assert_eq!(
        v["files_ok"].as_u64().unwrap() + 1,
        v["files_in_manifest"].as_u64().unwrap()
    );
}

#[test]
fn an_interrupted_offload_resumes_without_redoing_or_leaving_parts() {
    let f = fixture();
    let out = offload(&f)
        .env("NOLGIA_DIT_TEST_STOP_AFTER_FILES", "3")
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(130),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
    assert_eq!(json(&out)["status"], "interrupted");
    let out = offload(&f).output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v = json(&out);
    let resumed: u64 = v["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["resumed"].as_u64().unwrap())
        .sum();
    assert!(
        resumed >= 3,
        "the files finished before the stop are not copied again: {v}"
    );
    for d in [&f.d1, &f.d2] {
        let all = tree(&d.join("TEST"));
        assert!(
            all.keys().all(|k| !k.ends_with(".nolgia-part")),
            "no partial files left"
        );
        assert_eq!(tree(&d.join("TEST/B001")), tree(&f.canon));
    }
}

#[test]
fn a_different_file_at_the_destination_is_never_replaced() {
    let f = fixture();
    let there = f.d1.join("TEST/A001/PRIVATE/M4ROOT/CLIP/C0001.MP4");
    fs::create_dir_all(there.parent().unwrap()).unwrap();
    fs::write(&there, b"someone else's file").unwrap();
    let out = offload(&f).output().unwrap();
    assert_eq!(out.status.code(), Some(68));
    let v = json(&out);
    assert!(
        v["conflicts"][0]["path"]
            .as_str()
            .unwrap()
            .ends_with("C0001.MP4"),
        "{v}"
    );
    assert_eq!(fs::read(&there).unwrap(), b"someone else's file");
    assert!(
        !f.d2.join("TEST/A001").exists(),
        "nothing is copied while a conflict stands"
    );

    // Same size, different content: also refused, also untouched.
    let size = fs::metadata(f.sony.join("PRIVATE/M4ROOT/CLIP/C0001.MP4"))
        .unwrap()
        .len() as usize;
    fs::write(&there, vec![0u8; size]).unwrap();
    let out = offload(&f).output().unwrap();
    assert!(!out.status.success());
    assert_eq!(fs::read(&there).unwrap(), vec![0u8; size]);
}

#[test]
fn an_identical_file_already_there_counts_as_done() {
    let f = fixture();
    let src = f.canon.join("DCIM/100EOSR6/GK1A0001.CR3");
    let there = f.d1.join("TEST/B001/DCIM/100EOSR6/GK1A0001.CR3");
    fs::create_dir_all(there.parent().unwrap()).unwrap();
    fs::copy(&src, &there).unwrap();
    let out = offload(&f).output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v = json(&out);
    let already: u64 = v["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["already_there"].as_u64().unwrap())
        .sum();
    assert_eq!(already, 1);
}

#[test]
fn rename_then_undo_round_trips_and_a_second_rename_has_nothing_to_do() {
    let f = fixture();
    assert!(offload(&f).output().unwrap().status.success());
    let shoot = f.d1.join("TEST");
    let before = tree(&shoot);
    let args = |c: &mut Command| {
        c.args(["--json", "dit", "rename"]).arg(&shoot).args([
            "--shoot",
            "RX_BTS",
            "--camera",
            "ILME-FX3A=A-FX3A",
            "--clock-offset",
            "ILME-FX3A=-12h",
        ]);
    };
    let mut dry = nolgia();
    args(&mut dry);
    let out = dry.output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v = json(&out);
    assert_eq!(v["status"], "dry-run");
    assert_eq!(tree(&shoot), before, "a dry run changes nothing");

    let mut apply = nolgia();
    args(&mut apply);
    let out = apply.arg("--apply").output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(json(&out)["status"], "applied");
    let clip = shoot.join("A001/PRIVATE/M4ROOT/CLIP");
    assert!(
        clip.join("RX_BTS_20261012_1301_A-FX3A_C0001.MP4").is_file(),
        "the clock offset moved 01:01 the next day to 13:01"
    );
    assert!(
        clip.join("RX_BTS_20261012_1301_A-FX3A_C0001.XML").is_file(),
        "the sidecar follows its clip"
    );
    assert!(
        shoot
            .join("B001/DCIM/100EOSR6/RX_BTS_20261012_1327_B-R6m2_GK1A0001.CR3")
            .is_file()
    );
    assert!(
        shoot.join("A001/PRIVATE/M4ROOT/MEDIAPRO.XML").is_file(),
        "card files keep their names"
    );
    assert!(
        shoot
            .join("A001/PRIVATE/M4ROOT/THMBNL/C0001T01.JPG")
            .is_file()
    );
    let map = fs::read_to_string(shoot.join("_DIT/rename-map.csv")).unwrap();
    assert!(
        map.lines().count() >= 10,
        "one map line per renamed file: {map}"
    );

    let mut again = nolgia();
    args(&mut again);
    let out = again.output().unwrap();
    assert_eq!(json(&out)["status"], "nothing-to-do");

    let out = nolgia()
        .args(["--json", "dit", "undo"])
        .arg(&shoot)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        tree(&shoot),
        before,
        "undo restores every name and every byte"
    );
}

#[test]
fn inventory_flags_a_camera_clock_that_is_twelve_hours_off() {
    let f = fixture();
    let out = nolgia()
        .args(["--json", "dit", "inventory"])
        .arg(f.sony.parent().unwrap())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v = json(&out);
    let warnings = v["warnings"].to_string();
    assert!(warnings.contains("12h"), "{warnings}");
    let sony = v["cameras"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["camera"] == "ILME-FX3A")
        .unwrap();
    assert_eq!(sony["videos"], 3);
    assert_eq!(sony["gamma"][0], "s-log3-cine");

    let out = nolgia()
        .args([
            "--json",
            "dit",
            "inventory",
            "--clock-offset",
            "ILME-FX3A=-12h",
        ])
        .arg(f.sony.parent().unwrap())
        .output()
        .unwrap();
    assert!(
        !json(&out)["warnings"].to_string().contains("12h"),
        "corrected clocks line up"
    );
}

#[test]
fn a_dry_run_offload_copies_nothing() {
    let f = fixture();
    let out = offload(&f).arg("--dry-run").output().unwrap();
    assert!(out.status.success());
    let v = json(&out);
    assert_eq!(v["files"], 14);
    assert!(fs::read_dir(&f.d1).unwrap().next().is_none());
}

fn mhl_generations(card_copy: &Path) -> usize {
    fs::read_dir(card_copy.join("ascmhl"))
        .unwrap()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().ends_with(".mhl"))
        .count()
}

#[test]
fn running_a_finished_offload_again_copies_nothing_and_adds_no_generation() {
    let f = fixture();
    assert!(offload(&f).output().unwrap().status.success());
    let out = offload(&f).output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v = json(&out);
    assert_eq!(v["status"], "complete");
    assert_eq!(v["files_done_earlier"], v["files_verified"], "{v}");
    assert_eq!(v["bytes_copied"], 0);
    for d in [&f.d1, &f.d2] {
        for card in ["A001", "B001"] {
            assert_eq!(
                mhl_generations(&d.join("TEST").join(card)),
                1,
                "no second \"original\" generation for {card} at {}",
                d.display()
            );
        }
    }
    let out = nolgia()
        .args(["dit", "offload"])
        .arg(&f.sony)
        .arg(&f.canon)
        .arg("--to")
        .arg(&f.d1)
        .arg("--to")
        .arg(&f.d2)
        .args(["--shoot", "TEST", "--hash", "xxh64,md5"])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("nothing new to copy"), "{text}");
}

#[test]
fn a_shoot_folder_verifies_through_each_card_manifest() {
    let f = fixture();
    assert!(offload(&f).output().unwrap().status.success());
    let shoot = f.d2.join("TEST");
    let out = nolgia()
        .args(["--json", "dit", "verify"])
        .arg(&shoot)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
    let v = json(&out);
    assert_eq!(v["status"], "ok");
    assert_eq!(v["files_ok"], v["files_in_manifest"]);
    assert_eq!(v["files_in_manifest"], 14, "{v}");
    assert_eq!(v["not_in_manifest"].as_array().unwrap().len(), 0, "{v}");

    let out = nolgia()
        .args(["--json", "dit", "report"])
        .arg(&shoot)
        .arg("--verify")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(json(&out)["verification"]["status"], "ok");

    let victim = shoot.join("B001/DCIM/100EOSR6/GK1A0002.CR3");
    let mut bytes = fs::read(&victim).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 0x01;
    fs::write(&victim, bytes).unwrap();
    let out = nolgia()
        .args(["--json", "dit", "verify", "--record"])
        .arg(&shoot)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(68));
    let v = json(&out);
    assert_eq!(
        v["mismatched"][0]["path"],
        "B001/DCIM/100EOSR6/GK1A0002.CR3"
    );
    assert_eq!(
        v["recorded_generations"].as_array().unwrap().len(),
        2,
        "{v}"
    );
    assert_eq!(mhl_generations(&shoot.join("A001")), 2);
    assert_eq!(mhl_generations(&shoot.join("B001")), 2);
    // Each card keeps its own history: A001 stays good, and B001's failed
    // generation does not replace the hash it is checked against.
    let card = |c: &str| {
        nolgia()
            .args(["--json", "dit", "verify"])
            .arg(shoot.join(c))
            .output()
            .unwrap()
            .status
            .code()
    };
    assert_eq!(card("A001"), Some(0));
    assert_eq!(card("B001"), Some(68));
}

#[test]
fn status_finds_the_offload_from_the_destination() {
    let f = fixture();
    assert!(offload(&f).output().unwrap().status.success());
    let out = nolgia()
        .args(["--json", "dit", "status"])
        .arg(&f.d1)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v = json(&out);
    assert_eq!(v["files_done"], v["files_total"]);
    assert!(
        Path::new(v["folder"].as_str().unwrap()).ends_with("TEST"),
        "{v}"
    );
    let out = nolgia()
        .args(["dit", "status"])
        .arg(&f.d1)
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        text.ends_with("finished\n") || text.trim_end().ends_with("finished"),
        "{text}"
    );
}

/// A plain MP4 with no sidecar: `frames` of 1920x1080 H.264 at
/// 24000/1001 fps, and a stereo AAC track.
fn plain_mp4(frames: u32, seed: u64) -> Vec<u8> {
    let full = |vf: u32, body: &[u8]| {
        let mut v = vf.to_be_bytes().to_vec();
        v.extend(body);
        v
    };
    // Version and flags, creation and modification times, then the timescale.
    let mut mvhd = vec![0u8; 12];
    mvhd.extend(1000u32.to_be_bytes());
    mvhd.extend((frames * 1001 / 24).to_be_bytes());
    mvhd.resize(100, 0);
    let mdhd = |scale: u32, dur: u32| {
        let mut b = vec![0u8; 8];
        b.extend(scale.to_be_bytes());
        b.extend(dur.to_be_bytes());
        b.extend([0u8; 4]);
        full(0, &b)
    };
    let hdlr = |kind: &[u8; 4]| {
        let mut b = vec![0u8; 4];
        b.extend(kind);
        b.extend([0u8; 13]);
        full(0, &b)
    };
    let mut avc1 = vec![0u8; 6];
    avc1.extend(1u16.to_be_bytes());
    avc1.extend([0u8; 16]);
    avc1.extend(1920u16.to_be_bytes());
    avc1.extend(1080u16.to_be_bytes());
    avc1.extend([0u8; 50]);
    let mut stsd_v = 1u32.to_be_bytes().to_vec();
    stsd_v.extend(bx(b"avc1", &avc1));
    let mut stts = 1u32.to_be_bytes().to_vec();
    stts.extend(frames.to_be_bytes());
    stts.extend(1001u32.to_be_bytes());
    let video = bx(
        b"trak",
        &bx(
            b"mdia",
            &[
                bx(b"mdhd", &mdhd(24000, frames * 1001)),
                bx(b"hdlr", &hdlr(b"vide")),
                bx(
                    b"minf",
                    &bx(
                        b"stbl",
                        &[bx(b"stsd", &full(0, &stsd_v)), bx(b"stts", &full(0, &stts))].concat(),
                    ),
                ),
            ]
            .concat(),
        ),
    );
    let mut mp4a = vec![0u8; 6];
    mp4a.extend(1u16.to_be_bytes());
    mp4a.extend([0u8; 8]);
    mp4a.extend(2u16.to_be_bytes());
    mp4a.extend(16u16.to_be_bytes());
    mp4a.extend([0u8; 8]);
    let mut stsd_a = 1u32.to_be_bytes().to_vec();
    stsd_a.extend(bx(b"mp4a", &mp4a));
    let audio = bx(
        b"trak",
        &bx(
            b"mdia",
            &[
                bx(b"mdhd", &mdhd(48000, 0)),
                bx(b"hdlr", &hdlr(b"soun")),
                bx(b"minf", &bx(b"stbl", &bx(b"stsd", &full(0, &stsd_a)))),
            ]
            .concat(),
        ),
    );
    let mut f = bx(b"ftyp", b"isom\0\0\x02\0isomiso2avc1mp41");
    f.extend(bx(b"mdat", &noise(seed, 4096)));
    f.extend(bx(b"moov", &[bx(b"mvhd", &mvhd), video, audio].concat()));
    f
}

#[test]
fn inventory_reads_a_plain_mp4_without_ffprobe() {
    let tmp = tempfile::tempdir().unwrap();
    let card = tmp.path().join("A001_CARD");
    fs::create_dir_all(&card).unwrap();
    fs::write(card.join("C0001.MP4"), plain_mp4(48, 1)).unwrap();
    fs::write(card.join("C0002.MP4"), plain_mp4(240, 2)).unwrap();
    // No ffprobe or exiftool: only the built-in readers.
    let bare = || {
        let mut c = nolgia();
        c.env("PATH", tmp.path());
        c
    };
    let out = bare()
        .args(["--json", "dit", "inventory", "--files"])
        .arg(&card)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v = json(&out);
    let cam = &v["cameras"][0];
    assert_eq!(cam["camera"], "unknown");
    assert_eq!(cam["videos"], 2);
    assert_eq!(cam["video_seconds"], 12.0, "{cam}");
    assert_eq!(cam["frame_rates"][0], "23.976");
    assert_eq!(cam["resolutions"][0], "1920x1080");
    assert_eq!(cam["codecs"][0], "h264");
    assert_eq!(cam["times_from_file"], 2);
    let item = &v["items"][0];
    assert_eq!(item["path"], "C0001.MP4");
    assert_eq!(item["duration_s"], 2.0);
    assert_eq!(item["audio_channels"], 2);
    assert_eq!(item["source"], "mp4+file-time");

    let out = bare()
        .args(["dit", "inventory", "--files"])
        .arg(&card)
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    for want in [
        "unknown camera: 2 videos (12 s of footage)",
        "(file times: the files record no capture time)",
        "1920x1080, 23.976 fps, h264",
        "C0002.MP4: ",
        "source: mp4+file-time",
    ] {
        assert!(text.contains(want), "missing {want:?} in:\n{text}");
    }
}

#[test]
fn after_a_rename_the_manifest_records_it_and_the_shoot_still_verifies() {
    let f = fixture();
    assert!(offload(&f).output().unwrap().status.success());
    let shoot = f.d1.join("TEST");
    let rename = |extra: &[&str]| {
        nolgia()
            .args(["--json", "dit", "rename"])
            .arg(&shoot)
            .args(["--shoot", "RX_BTS"])
            .args(extra)
            .output()
            .unwrap()
    };
    let out = rename(&["--apply"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let renamed = json(&out)["to_rename"].as_u64().unwrap();
    assert!(renamed > 0);
    let verify = || {
        nolgia()
            .args(["--json", "dit", "verify"])
            .arg(&shoot)
            .output()
            .unwrap()
    };
    assert_eq!(
        verify().status.code(),
        Some(68),
        "the history still lists the old names"
    );
    let manifest = |card: &str| {
        let out = nolgia()
            .args(["--json", "dit", "manifest"])
            .arg(shoot.join(card))
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stdout)
        );
        json(&out)
    };
    let mut found = 0;
    for card in ["A001", "B001"] {
        let v = manifest(card);
        assert_eq!(v["missing"].as_array().unwrap().len(), 0, "{v}");
        assert_eq!(v["differ"].as_array().unwrap().len(), 0, "{v}");
        found += v["renamed"].as_array().unwrap().len() as u64;
    }
    assert_eq!(found, renamed, "every renamed file is found by its hashes");
    let gen2 = fs::read_dir(shoot.join("A001/ascmhl"))
        .unwrap()
        .flatten()
        .find(|e| e.file_name().to_string_lossy().starts_with("0002_"))
        .unwrap();
    assert!(
        fs::read_to_string(gen2.path())
            .unwrap()
            .contains("<previousPath>PRIVATE/M4ROOT/CLIP/C0001.MP4</previousPath>")
    );
    let out = verify();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );

    // Undo, record again, and the original names verify.
    let out = nolgia()
        .args(["--json", "dit", "undo"])
        .arg(&shoot)
        .output()
        .unwrap();
    assert!(out.status.success());
    assert!(json(&out)["warnings"].to_string().contains("dit manifest"));
    assert_eq!(verify().status.code(), Some(68));
    for card in ["A001", "B001"] {
        manifest(card);
    }
    assert!(verify().status.success());
}

/// The Mac test 2 report: an agent passed the folder holding both cards
/// (`cards/` with `A001/` and `B001/`) as one source. Each card must still
/// land in its own card folder, as if they had been named one by one.
#[test]
fn a_folder_holding_two_cards_is_offloaded_card_by_card() {
    let f = fixture();
    let cards_dir = f.sony.parent().unwrap().to_path_buf();
    // macOS leaves a .DS_Store in the folder; it does not stop the split.
    fs::write(cards_dir.join(".DS_Store"), "x").unwrap();
    let out = nolgia()
        .args(["--json", "dit", "offload"])
        .arg(&cards_dir)
        .arg("--to")
        .arg(&f.d1)
        .arg("--to")
        .arg(&f.d2)
        .args(["--shoot", "TEST"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v = json(&out);
    assert_eq!(v["status"], "complete");
    let cards: Vec<&str> = v["plan"]["cards"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["card"].as_str().unwrap())
        .collect();
    assert_eq!(cards, ["A001", "B001"], "{v}");
    assert_eq!(
        v["plan"]["split_sources"][0]["cards"],
        serde_json::json!(["A001", "B001"])
    );
    for d in [&f.d1, &f.d2] {
        assert!(d.join("TEST/A001/PRIVATE/M4ROOT/CLIP/C0001.MP4").is_file());
        assert!(d.join("TEST/B001/DCIM/100EOSR6/GK1A0001.CR3").is_file());
        assert!(
            !d.join("TEST/cards").exists(),
            "no card named after the folder"
        );
        assert!(d.join("TEST/A001/ascmhl/ascmhl_chain.xml").is_file());
        assert!(d.join("TEST/B001/ascmhl/ascmhl_chain.xml").is_file());
    }
    // The text output says what happened.
    let text = nolgia()
        .args(["dit", "offload"])
        .arg(&cards_dir)
        .arg("--to")
        .arg(&f.d1)
        .args(["--shoot", "TEST", "--dry-run"])
        .output()
        .unwrap();
    let t = String::from_utf8_lossy(&text.stdout);
    assert!(
        t.contains("holds 2 cards (A001, B001): each is offloaded as its own card"),
        "{t}"
    );
}

#[test]
fn a_folder_with_cards_and_other_files_is_copied_whole_with_a_note() {
    let f = fixture();
    let cards_dir = f.sony.parent().unwrap().to_path_buf();
    fs::write(cards_dir.join("camera-notes.txt"), "roll 1").unwrap();
    let out = nolgia()
        .args(["--json", "dit", "offload"])
        .arg(&cards_dir)
        .arg("--to")
        .arg(&f.d1)
        .args(["--shoot", "TEST", "--dry-run"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v = json(&out);
    assert_eq!(v["cards"][0]["card"], "cards", "{v}");
    assert!(v.get("split_sources").is_none(), "{v}");
    let warned = v["warnings"].as_array().unwrap().iter().any(|w| {
        w.as_str()
            .unwrap()
            .contains("holds the cards A001, B001 beside other things (camera-notes.txt)")
    });
    assert!(warned, "{v}");
}

#[test]
fn a_card_named_with_card_is_never_split() {
    let f = fixture();
    let cards_dir = f.sony.parent().unwrap().to_path_buf();
    let out = nolgia()
        .args(["--json", "dit", "offload"])
        .arg(&cards_dir)
        .arg("--to")
        .arg(&f.d1)
        .args(["--card", "ROLL1", "--dry-run"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v = json(&out);
    assert_eq!(v["cards"].as_array().unwrap().len(), 1, "{v}");
    assert_eq!(v["cards"][0]["card"], "ROLL1");
}

/// The Mac test 2 report: `report --verify` in the same second as the
/// offload took the offload's report name and replaced it.
#[test]
fn a_report_never_replaces_an_earlier_report() {
    let f = fixture();
    let out = offload(&f).output().unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let first = PathBuf::from(json(&out)["reports"][0].as_str().unwrap().to_string());
    let first_text = fs::read_to_string(&first).unwrap();
    let shoot = f.d1.join("TEST");
    let mut made = vec![first.clone()];
    // Several reports as fast as possible: many land in the offload's second.
    for _ in 0..3 {
        let out = nolgia()
            .args(["--json", "dit", "report", "--verify"])
            .arg(&shoot)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "stderr: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let md = PathBuf::from(json(&out)["markdown"].as_str().unwrap().to_string());
        assert!(!made.contains(&md), "{} was used twice", md.display());
        assert!(md.with_extension("json").is_file());
        made.push(md);
    }
    assert_eq!(
        fs::read_to_string(&first).unwrap(),
        first_text,
        "the offload's report is untouched"
    );
    let reports = fs::read_dir(shoot.join("_DIT"))
        .unwrap()
        .flatten()
        .filter(|e| {
            let n = e.file_name().to_string_lossy().to_string();
            n.starts_with("DIT-REPORT-") && n.ends_with(".md")
        })
        .count();
    assert_eq!(reports, 4);
}
