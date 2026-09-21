use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::Value;
use std::path::Path;

fn install(dir: &Path) -> Command {
    let mut command = Command::cargo_bin("nolgia").unwrap();
    command
        .env_remove("NOLGIA_TOKEN")
        .env("NOLGIA_TOKEN_STORE", "file")
        .env("XDG_CONFIG_HOME", dir)
        .env("XDG_STATE_HOME", dir)
        .env("NOLGIA_NO_UPDATE_CHECK", "1")
        .args(["skills", "install", "--dir"])
        .arg(dir);
    command
}

#[test]
fn installation_reports_each_pack_and_repeated_runs_preserve_files() {
    let dir = tempfile::tempdir().unwrap();
    let installed = install(dir.path()).arg("--json").assert().success();
    let packs: Vec<Value> = serde_json::from_slice(&installed.get_output().stdout).unwrap();
    assert_eq!(packs.len(), 3);
    for pack in &packs {
        assert_eq!(pack["status"], "installed");
        assert!(Path::new(pack["path"].as_str().unwrap()).is_file());
    }

    install(dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "unchanged nolgia-platform (already at ",
        ))
        .stdout(predicate::str::contains(
            "0 installed, 3 unchanged, 0 skipped, 0 overwritten",
        ));

    let path = dir.path().join("nolgia-platform/SKILL.md");
    std::fs::write(&path, "customized skill").unwrap();
    install(dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("skipped nolgia-platform (differs from the bundled copy; pass --force to update it) -> "))
        .stdout(predicate::str::contains("0 installed, 2 unchanged, 1 skipped, 0 overwritten"));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "customized skill");

    let forced = install(dir.path())
        .args(["--force", "--json"])
        .assert()
        .success();
    let packs: Vec<Value> = serde_json::from_slice(&forced.get_output().stdout).unwrap();
    assert_eq!(packs[0]["status"], "overwritten");
    assert!(packs[1..].iter().all(|pack| pack["status"] == "unchanged"));
    assert!(std::fs::read_to_string(&path).unwrap().starts_with("---\n"));
}

#[test]
fn partially_installed_machine_installs_remaining_packs_without_force() {
    let dir = tempfile::tempdir().unwrap();
    let pack_dir = dir.path().join("nolgia-platform");
    std::fs::create_dir(&pack_dir).unwrap();
    std::fs::write(pack_dir.join("SKILL.md"), "customized skill").unwrap();

    let result = install(dir.path()).arg("--json").assert().success();

    let packs: Vec<Value> = serde_json::from_slice(&result.get_output().stdout).unwrap();
    assert_eq!(packs[0]["status"], "skipped");
    assert!(packs[1..].iter().all(|pack| pack["status"] == "installed"));
    assert_eq!(
        std::fs::read_to_string(pack_dir.join("SKILL.md")).unwrap(),
        "customized skill"
    );
}

#[test]
fn filesystem_errors_still_fail_installation() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("nolgia-platform"), "not a directory").unwrap();

    install(dir.path()).assert().code(1);
}
