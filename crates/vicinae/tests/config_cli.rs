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
    let settings = "// written by the C++ engine\n{ \"keybinding\": \"emacs\", \"tray\": { \"enabled\": false }, \"pop_to_root_on_close\": true }\n";
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
    assert!(report.contains("tray.enabled -> tray.enabled"), "{report}");
    assert!(
        report.contains("pop_to_root_on_close: no vicinae.json equivalent"),
        "{report}"
    );

    assert!(
        run(home.path(), &["config", "migrate", "--write"])
            .status
            .success()
    );
    let written = compass_core::Config::load_from(dir.join("vicinae.json")).unwrap();
    assert_eq!(written.launcher().keybinding(), "emacs");
    assert!(!written.tray().enabled());
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

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

#[test]
fn config_default_prints_every_default_as_json() {
    let home = tempfile::tempdir().unwrap();
    let out = run(home.path(), &["config", "default"]);
    assert!(out.status.success());
    let printed: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(printed, compass_core::config::default_document());
}

#[test]
fn script_template_prints_the_cpp_template_and_script_check_reads_it_back() {
    let home = tempfile::tempdir().unwrap();
    let out = run(
        home.path(),
        &[
            "script", "template", "-t", "Say Hi", "-l", "python", "-m", "compact",
        ],
    );
    assert!(out.status.success(), "{}", text(&out.stderr));
    let script = text(&out.stdout);
    assert_eq!(
        script,
        format!(
            "{}\n",
            compass_core::script_template::generate(
                "Say Hi",
                compass_core::script_template::Language::Python,
                compass_core::script_command::OutputMode::Compact,
            )
        )
    );
    let file = home.path().join("say-hi.py");
    std::fs::write(&file, &script).unwrap();
    let checked = run(home.path(), &["script", "check", file.to_str().unwrap()]);
    assert!(checked.status.success(), "{}", text(&checked.stderr));
    assert!(checked.stdout.is_empty() && checked.stderr.is_empty());

    std::fs::write(&file, "#!/bin/sh\n# @vicinae.schemaVersion 1\n").unwrap();
    let broken = run(home.path(), &["script", "check", file.to_str().unwrap()]);
    assert_eq!(broken.status.code(), Some(1));
    assert!(text(&broken.stderr).starts_with("Error: "));

    let missing = run(home.path(), &["script", "check", "/nonexistent/x.sh"]);
    assert_eq!(missing.status.code(), Some(1));
    assert!(text(&missing.stderr).contains("Error: File not found: /nonexistent/x.sh"));

    let cobol = run(
        home.path(),
        &["script", "template", "-t", "x", "-l", "cobol"],
    );
    assert_eq!(cobol.status.code(), Some(1));
    assert!(
        text(&cobol.stderr)
            .contains("Invalid language: cobol\n\nSupported languages: bash, python, javascript")
    );
    let mode = run(
        home.path(),
        &["script", "template", "-t", "x", "-m", "loud"],
    );
    assert!(text(&mode.stderr).contains("Supported modes: fullOutput, compact"));
}

#[test]
fn theme_template_check_and_paths() {
    let home = tempfile::tempdir().unwrap();
    let template = run(home.path(), &["theme", "template"]);
    assert!(template.status.success());
    let expected = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../extra/theme-template.toml"),
    )
    .unwrap();
    assert_eq!(text(&template.stdout), format!("{expected}\n"));

    let file = home.path().join("mine.toml");
    std::fs::write(&file, &template.stdout).unwrap();
    let valid = run(home.path(), &["th", "check", file.to_str().unwrap()]);
    assert!(valid.status.success(), "{}", text(&valid.stderr));
    assert_eq!(text(&valid.stdout), "Theme file is valid\n");

    std::fs::write(&file, "[colors.core]\naccent = \"#fff\"\n").unwrap();
    let invalid = run(home.path(), &["theme", "check", file.to_str().unwrap()]);
    assert_eq!(invalid.status.code(), Some(1));
    assert!(text(&invalid.stderr).contains("Theme is invalid: a [meta] table is required"));

    let paths = run(home.path(), &["theme", "paths"]);
    assert!(paths.status.success());
    assert_eq!(
        text(&paths.stdout).lines().next(),
        Some(
            home.path()
                .join(".local/share/vicinae/themes")
                .to_str()
                .unwrap()
        )
    );
}

#[test]
fn version_prints_the_cpps_three_lines() {
    let home = tempfile::tempdir().unwrap();
    let out = run(home.path(), &["version"]);
    assert!(out.status.success());
    let printed = text(&out.stdout);
    assert!(printed.starts_with(&format!("Version {} (commit ", env!("CARGO_PKG_VERSION"))));
    assert_eq!(printed.lines().count(), 3);
    assert_eq!(text(&run(home.path(), &["ver"]).stdout), printed);
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
