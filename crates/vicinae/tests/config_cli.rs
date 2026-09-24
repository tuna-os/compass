//! `vicinae config`: the real binary against a throwaway `$XDG_CONFIG_HOME`.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn run(config_home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_vicinae"))
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", config_home)
        .env("XDG_CONFIG_HOME", config_home)
        .args(args)
        .output()
        .expect("the CLI binary should run")
}

fn published_schema() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../packaging/schema/vicinae.schema.json")
}

#[test]
fn schema_prints_the_published_schema() {
    let home = tempfile::tempdir().unwrap();
    let out = run(home.path(), &["config", "schema"]);
    assert!(out.status.success());
    assert_eq!(
        String::from_utf8(out.stdout).unwrap(),
        std::fs::read_to_string(published_schema()).unwrap()
    );
}

#[test]
fn migrate_writes_vicinae_json_and_leaves_the_cpp_file_alone() {
    let home = tempfile::tempdir().unwrap();
    let dir = home.path().join("vicinae");
    std::fs::create_dir_all(&dir).unwrap();
    let settings = "// written by the C++ engine\n{ \"keybinding\": \"emacs\", \"tray\": { \"enabled\": false } }\n";
    std::fs::write(dir.join("settings.json"), settings).unwrap();

    let dry = run(home.path(), &["config", "migrate"]);
    assert!(dry.status.success());
    assert!(
        !dir.join("vicinae.json").exists(),
        "a dry run writes nothing"
    );
    let report = String::from_utf8(dry.stdout).unwrap();
    assert!(
        report.contains("keybinding -> launcher.keybinding"),
        "{report}"
    );
    assert!(
        report.contains("tray.enabled: no vicinae.json equivalent"),
        "{report}"
    );

    assert!(
        run(home.path(), &["config", "migrate", "--write"])
            .status
            .success()
    );
    let written = compass_core::Config::load_from(dir.join("vicinae.json")).unwrap();
    assert_eq!(written.launcher().keybinding(), "emacs");
    assert_eq!(
        std::fs::read_to_string(dir.join("settings.json")).unwrap(),
        settings
    );

    let again = run(home.path(), &["config", "migrate", "--write"]);
    assert!(
        !again.status.success(),
        "an existing vicinae.json is not replaced silently"
    );
    assert!(
        run(home.path(), &["config", "migrate", "--write", "--force"])
            .status
            .success()
    );
    assert!(dir.join("vicinae.json.bak").exists());
}

#[test]
fn migrate_without_settings_fails_clearly() {
    let home = tempfile::tempdir().unwrap();
    let out = run(home.path(), &["config", "migrate"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(
        String::from_utf8(out.stderr)
            .unwrap()
            .contains("no settings to migrate")
    );
}
