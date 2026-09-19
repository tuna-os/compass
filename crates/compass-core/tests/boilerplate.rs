//! What the boilerplate generator writes, and what it refuses to write.
//!
//! Read off `ExtensionBoilerplateGenerator::generate`
//! (`src/server/src/services/extension-boilerplate-generator/`).

use std::fs;
use std::path::Path;

use compass_core::boilerplate::{
    COMMAND_BOILERPLATES, CommandConfig, CommandMode, Config, api_dependency_version,
    boilerplate_by_resource, generate, simplified,
};

/// A config that generates cleanly, for tests that vary one thing about it.
fn a_config() -> Config {
    Config {
        author: "ada".to_string(),
        title: "My Extension".to_string(),
        description: "Does a useful thing".to_string(),
        commands: vec![CommandConfig {
            title: "Show Things".to_string(),
            description: "Shows the things".to_string(),
            template_id: ":boilerplate/tmpl-list".to_string(),
        }],
    }
}

/// The manifest of the extension generated under `dir`, parsed.
fn manifest_of(ext_dir: &Path) -> serde_json::Value {
    let text = fs::read_to_string(ext_dir.join("package.json")).expect("manifest written");
    serde_json::from_str(&text).expect("manifest is valid JSON")
}

#[test]
fn the_extension_directory_is_the_title_slugified() {
    let target = tempfile::tempdir().expect("tempdir");
    let ext_dir = generate(target.path(), &a_config(), "v1.2.3").expect("generates");
    assert_eq!(ext_dir, target.path().join("my-extension"));
    assert!(ext_dir.is_dir());
}

#[test]
fn a_target_that_is_not_a_directory_is_refused_by_name() {
    let target = tempfile::tempdir().expect("tempdir");
    let missing = target.path().join("nope");
    let err = generate(&missing, &a_config(), "v1.2.3").expect_err("refuses");
    assert!(err.contains(&missing.display().to_string()), "{err}");
    assert!(
        err.contains("will not create the containing directory for you"),
        "the message has to say the generator won't make it, or the person \
         will just try again: {err}"
    );
}

#[test]
fn an_existing_extension_directory_is_never_overwritten() {
    let target = tempfile::tempdir().expect("tempdir");
    let ext_dir = target.path().join("my-extension");
    fs::create_dir(&ext_dir).expect("pre-existing");
    fs::write(ext_dir.join("package.json"), "mine").expect("pre-existing file");

    let err = generate(target.path(), &a_config(), "v1.2.3").expect_err("refuses");
    assert!(err.contains("Won't override."), "{err}");
    assert_eq!(
        fs::read_to_string(ext_dir.join("package.json")).expect("still there"),
        "mine",
        "refusing has to mean leaving the person's files alone"
    );
}

#[test]
fn an_unknown_template_id_is_named_in_the_error() {
    let target = tempfile::tempdir().expect("tempdir");
    let mut config = a_config();
    config.commands[0].template_id = ":boilerplate/tmpl-nonexistent".to_string();

    let err = generate(target.path(), &config, "v1.2.3").expect_err("refuses");
    assert_eq!(
        err,
        "Unknown template with id :boilerplate/tmpl-nonexistent"
    );
}

#[test]
fn the_source_file_is_named_after_the_command_and_typed_by_its_mode() {
    let target = tempfile::tempdir().expect("tempdir");
    let mut config = a_config();
    config.commands.push(CommandConfig {
        title: "Run Quietly".to_string(),
        description: "No view here".to_string(),
        template_id: ":boilerplate/tmpl-no-view".to_string(),
    });

    let ext_dir = generate(target.path(), &config, "v1.2.3").expect("generates");
    assert!(
        ext_dir.join("src/show-things.tsx").is_file(),
        "a view command is TSX"
    );
    assert!(
        ext_dir.join("src/run-quietly.ts").is_file(),
        "a no-view command is TS"
    );
}

#[test]
fn the_source_file_holds_the_templates_own_contents() {
    let target = tempfile::tempdir().expect("tempdir");
    let ext_dir = generate(target.path(), &a_config(), "v1.2.3").expect("generates");
    let written = fs::read_to_string(ext_dir.join("src/show-things.tsx")).expect("source written");
    let template = boilerplate_by_resource(":boilerplate/tmpl-list").expect("template exists");
    assert_eq!(written, template.source);
}

#[test]
fn the_manifest_carries_the_config_through() {
    let target = tempfile::tempdir().expect("tempdir");
    let ext_dir = generate(target.path(), &a_config(), "v1.2.3").expect("generates");
    let manifest = manifest_of(&ext_dir);

    assert_eq!(manifest["name"], "my-extension");
    assert_eq!(manifest["title"], "My Extension");
    assert_eq!(manifest["description"], "Does a useful thing");
    assert_eq!(manifest["author"], "ada");
    assert_eq!(manifest["icon"], "extension_icon.png");
}

#[test]
fn each_command_becomes_one_manifest_entry() {
    let target = tempfile::tempdir().expect("tempdir");
    let mut config = a_config();
    config.commands.push(CommandConfig {
        title: "Run Quietly".to_string(),
        description: "No view here".to_string(),
        template_id: ":boilerplate/tmpl-no-view".to_string(),
    });

    let ext_dir = generate(target.path(), &config, "v1.2.3").expect("generates");
    let manifest = manifest_of(&ext_dir);
    let commands = manifest["commands"].as_array().expect("an array");

    assert_eq!(commands.len(), 2);
    assert_eq!(commands[0]["name"], "show-things");
    assert_eq!(commands[0]["title"], "Show Things");
    assert_eq!(commands[0]["description"], "Shows the things");
    assert_eq!(commands[0]["mode"], "view");
    assert_eq!(commands[1]["name"], "run-quietly");
    assert_eq!(commands[1]["mode"], "no-view");
}

#[test]
fn the_manifest_keeps_the_templates_key_order() {
    // The C++ templates the manifest as text precisely so a JSON round-trip
    // cannot reorder it; a generated manifest that came out alphabetised
    // would mean this port lost that.
    let target = tempfile::tempdir().expect("tempdir");
    let ext_dir = generate(target.path(), &a_config(), "v1.2.3").expect("generates");
    let text = fs::read_to_string(ext_dir.join("package.json")).expect("manifest written");

    let order: Vec<usize> = ["\"name\"", "\"title\"", "\"description\"", "\"categories\""]
        .iter()
        .map(|key| text.find(key).unwrap_or_else(|| panic!("{key} present")))
        .collect();
    assert!(
        order.windows(2).all(|pair| pair[0] < pair[1]),
        "manifest keys are out of template order:\n{text}"
    );
}

#[test]
fn a_release_tag_pins_the_api_to_a_caret_range() {
    assert_eq!(api_dependency_version("v1.2.3"), "^1.2.3");
}

#[test]
fn anything_that_is_not_a_release_tag_falls_back_to_latest() {
    // Not three parts.
    assert_eq!(api_dependency_version("v1.2"), "latest");
    assert_eq!(api_dependency_version("v1.2.3.4"), "latest");
    // No leading v.
    assert_eq!(api_dependency_version("1.2.3"), "latest");
    // A tag-less build.
    assert_eq!(api_dependency_version(""), "latest");
}

#[test]
fn the_version_reaches_the_generated_manifest() {
    let target = tempfile::tempdir().expect("tempdir");
    let ext_dir = generate(target.path(), &a_config(), "v0.9.1").expect("generates");
    let manifest = manifest_of(&ext_dir);
    assert_eq!(manifest["dependencies"]["@vicinae/api"], "^0.9.1");
}

#[test]
fn the_gitignore_hides_node_modules_and_the_generated_types() {
    let target = tempfile::tempdir().expect("tempdir");
    let ext_dir = generate(target.path(), &a_config(), "v1.2.3").expect("generates");
    let text = fs::read_to_string(ext_dir.join(".gitignore")).expect("gitignore written");
    assert_eq!(text, "node_modules\nvicinae-env.d.ts\n");
}

#[test]
fn the_supporting_files_are_all_written() {
    let target = tempfile::tempdir().expect("tempdir");
    let ext_dir = generate(target.path(), &a_config(), "v1.2.3").expect("generates");
    for relative in ["tsconfig.json", "README.md", "assets/extension_icon.png"] {
        assert!(ext_dir.join(relative).is_file(), "{relative} is missing");
    }
    let icon = fs::read(ext_dir.join("assets/extension_icon.png")).expect("icon written");
    assert!(
        icon.starts_with(b"\x89PNG"),
        "the icon has to be a real PNG"
    );
}

#[test]
fn whitespace_in_a_title_cannot_break_the_manifest_it_lands_in() {
    // Every substituted value is `simplified()` first, so a pasted newline
    // ends up as a space rather than an unescaped newline inside a JSON
    // string literal.
    let target = tempfile::tempdir().expect("tempdir");
    let mut config = a_config();
    config.description = "Does\na useful\tthing".to_string();

    let ext_dir = generate(target.path(), &config, "v1.2.3").expect("generates");
    // manifest_of parses, so a broken manifest fails here.
    assert_eq!(manifest_of(&ext_dir)["description"], "Does a useful thing");
}

#[test]
fn simplified_trims_and_collapses() {
    assert_eq!(simplified("  a  b \n c "), "a b c");
}

#[test]
fn an_extension_with_no_commands_still_generates() {
    let target = tempfile::tempdir().expect("tempdir");
    let mut config = a_config();
    config.commands.clear();

    let ext_dir = generate(target.path(), &config, "v1.2.3").expect("generates");
    let manifest = manifest_of(&ext_dir);
    assert_eq!(manifest["commands"].as_array().expect("an array").len(), 0);
}

#[test]
fn every_offered_template_generates() {
    // The picker offers these five; a resource id that no longer resolves
    // would make the form fail only once a person picked that entry.
    for tmpl in COMMAND_BOILERPLATES {
        let target = tempfile::tempdir().expect("tempdir");
        let mut config = a_config();
        config.commands[0].template_id = tmpl.resource.to_string();

        let ext_dir = generate(target.path(), &config, "v1.2.3")
            .unwrap_or_else(|err| panic!("{} should generate: {err}", tmpl.resource));
        let extension = tmpl.mode.source_extension();
        assert!(
            ext_dir
                .join(format!("src/show-things.{extension}"))
                .is_file()
        );
        assert!(!tmpl.source.is_empty(), "{} has no source", tmpl.resource);
    }
}

#[test]
fn a_no_view_template_is_the_only_one_that_is_not_a_view() {
    let modes: Vec<CommandMode> = COMMAND_BOILERPLATES.iter().map(|t| t.mode).collect();
    assert_eq!(
        modes.iter().filter(|m| **m == CommandMode::NoView).count(),
        1
    );
    assert_eq!(
        boilerplate_by_resource(":boilerplate/tmpl-no-view")
            .expect("exists")
            .mode,
        CommandMode::NoView
    );
}

#[cfg(unix)]
#[test]
fn copied_files_are_readable_and_writable_by_their_owner_only() {
    use std::os::unix::fs::PermissionsExt;

    let target = tempfile::tempdir().expect("tempdir");
    let ext_dir = generate(target.path(), &a_config(), "v1.2.3").expect("generates");
    for relative in ["tsconfig.json", "README.md", "src/show-things.tsx"] {
        let mode = fs::metadata(ext_dir.join(relative))
            .expect("written")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600, "{relative} has mode {mode:o}");
    }
}

#[test]
fn two_commands_that_slugify_alike_do_not_clobber_each_other() {
    // Qt's QFile::copy refuses an existing destination, so the first command's
    // file survives. Both still get a manifest entry, which is the odd part
    // and the reason this is pinned: the extension is generated, not rejected.
    let target = tempfile::tempdir().expect("tempdir");
    let mut config = a_config();
    config.commands.push(CommandConfig {
        title: "show things".to_string(),
        description: "The same slug".to_string(),
        template_id: ":boilerplate/tmpl-simple-detail".to_string(),
    });

    let ext_dir = generate(target.path(), &config, "v1.2.3").expect("generates");
    let written = fs::read_to_string(ext_dir.join("src/show-things.tsx")).expect("source written");
    let first = boilerplate_by_resource(":boilerplate/tmpl-list").expect("template exists");
    assert_eq!(written, first.source, "the first command's file wins");
    assert_eq!(
        manifest_of(&ext_dir)["commands"]
            .as_array()
            .expect("array")
            .len(),
        2
    );
}
