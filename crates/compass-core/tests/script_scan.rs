//! The script scan's rules, and what a found script runs and shows.
//!
//! Ported from `src/server/src/script/script-scanner.cpp`,
//! `script-command-file.cpp` and `script-metadata-store.cpp`.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use compass_core::image_url::{ColorLike, ImageUrlType};
use compass_core::script_scan::{
    DirEntryInfo, EntryOutcome, MAX_DEPTH, SNIFF_BYTES, ScannedDirectory, ScriptCommandFile,
    ScriptMetadataStore, classify, default_icon, entry_id, has_forbidden_extension,
    head_looks_like_text, is_skipped_name, last_path_component, percent_encode, scan_directories,
    url_scheme,
};

fn dir(id: &str, depth: u8) -> ScannedDirectory {
    ScannedDirectory {
        id: id.to_owned(),
        path: PathBuf::from("/scripts"),
        depth,
    }
}

fn file(name: &str) -> DirEntryInfo {
    DirEntryInfo {
        name: name.to_owned(),
        path: PathBuf::from("/scripts").join(name),
        is_dir: false,
    }
}

fn subdir(name: &str) -> DirEntryInfo {
    DirEntryInfo {
        name: name.to_owned(),
        path: PathBuf::from("/scripts").join(name),
        is_dir: true,
    }
}

fn text(_: &Path) -> bool {
    true
}

fn binary(_: &Path) -> bool {
    false
}

// --- names and ids ------------------------------------------------------

#[test]
fn a_dotfile_is_not_a_script() {
    assert!(is_skipped_name(".hidden.sh"));
}

#[test]
fn a_template_is_skipped_wherever_the_word_sits() {
    // The C++ uses `contains`, not `ends_with`: a Raycast template is named
    // `something.template.sh`, so the marker is in the middle.
    assert!(is_skipped_name("greet.template.sh"));
    assert!(is_skipped_name("greet.template"));
    assert!(is_skipped_name("x.templates"));
}

#[test]
fn an_ordinary_name_survives() {
    assert!(!is_skipped_name("greet.sh"));
    assert!(!is_skipped_name("template.sh"));
}

#[test]
fn a_root_directorys_entries_are_named_by_the_file_alone() {
    assert_eq!(entry_id("", "greet.sh"), "greet.sh");
}

#[test]
fn a_nested_entry_is_named_with_dots() {
    assert_eq!(entry_id("tools", "greet.sh"), "tools.greet.sh");
}

#[test]
fn a_trailing_separator_does_not_lose_the_name() {
    // `std::filesystem::path("/a/b/")` has no filename and the C++ helper
    // reaches for the parent to recover `b`. Rust's `Path` drops the trailing
    // separator itself, so the same answer arrives by a shorter route — but it
    // is the same answer, which is what the scan depends on.
    assert_eq!(last_path_component(Path::new("/a/b/")), "b");
    assert_eq!(last_path_component(Path::new("/a/b")), "b");
}

#[test]
fn a_path_with_no_name_at_all_falls_back_to_the_path_itself() {
    // This is the branch the C++ fallback really exists for: the root has no
    // filename and no parent, so the helper returns the path as written rather
    // than an empty id that would collide with every other nameless entry.
    assert_eq!(last_path_component(Path::new("/")), "/");
}

#[test]
fn a_relative_traversal_keeps_its_own_text() {
    // `..` has no filename either, and its parent is empty.
    assert_eq!(last_path_component(Path::new("..")), "..");
}

// --- extensions and binaries -------------------------------------------

#[test]
fn documentation_is_never_a_script() {
    assert!(has_forbidden_extension(Path::new("/s/README.md")));
    assert!(has_forbidden_extension(Path::new("/s/logo.svg")));
    assert!(has_forbidden_extension(Path::new("/s/notes.txt")));
}

#[test]
fn an_extensionless_file_is_allowed() {
    // Most shell scripts in a scripts directory have no extension at all.
    assert!(!has_forbidden_extension(Path::new("/s/greet")));
}

#[test]
fn the_extension_check_is_case_sensitive() {
    // `std::filesystem::path::extension` returns the text as written and the
    // C++ compares it as written, so `README.MD` is not filtered.
    assert!(!has_forbidden_extension(Path::new("/s/README.MD")));
}

#[test]
fn a_nul_byte_makes_a_file_binary() {
    assert!(!head_looks_like_text(b"ELF\0\x01"));
}

#[test]
fn an_empty_file_counts_as_text() {
    // There is no NUL in nothing, so an empty file reaches the parser — which
    // then rejects it for having no header, with a message that says so.
    assert!(head_looks_like_text(b""));
}

#[test]
fn a_nul_past_the_sniff_window_is_not_seen() {
    let mut head = vec![b'a'; SNIFF_BYTES];
    head.push(0);
    assert!(head_looks_like_text(&head));
}

#[test]
fn a_nul_at_the_last_sniffed_byte_is_seen() {
    let mut head = vec![b'a'; SNIFF_BYTES - 1];
    head.push(0);
    assert!(!head_looks_like_text(&head));
}

// --- the scan's order of checks ----------------------------------------

#[test]
fn a_hidden_entry_never_gets_an_id() {
    let seen = HashSet::new();
    assert_eq!(
        classify(&dir("", 0), &file(".git"), &seen, text),
        EntryOutcome::Hidden
    );
}

#[test]
fn a_directory_is_descended_with_its_own_id_as_the_prefix() {
    let seen = HashSet::new();
    let outcome = classify(&dir("tools", 1), &subdir("net"), &seen, text);
    let EntryOutcome::Descend(next) = outcome else {
        panic!("expected a descent, got {outcome:?}");
    };
    assert_eq!(next.id, "tools.net");
    assert_eq!(next.depth, 2);
}

#[test]
fn the_walk_stops_one_short_of_the_depth_cap() {
    // The C++ tests `depth + 1 < MAX_DEPTH`, so a directory at depth 4 is
    // listed but never opened.
    let seen = HashSet::new();
    let deep = dir("a.b.c.d", MAX_DEPTH - 1);
    assert_eq!(
        classify(&deep, &subdir("e"), &seen, text),
        EntryOutcome::TooDeep
    );
    let shallower = dir("a.b.c", MAX_DEPTH - 2);
    assert!(matches!(
        classify(&shallower, &subdir("d"), &seen, text),
        EntryOutcome::Descend(_)
    ));
}

#[test]
fn a_directory_does_not_consume_an_id() {
    // A directory is classified before the duplicate check, so a directory
    // named like an already-seen script still contributes its children.
    let mut seen = HashSet::new();
    seen.insert("greet".to_owned());
    assert!(matches!(
        classify(&dir("", 0), &subdir("greet"), &seen, text),
        EntryOutcome::Descend(_)
    ));
}

#[test]
fn the_first_script_with_an_id_wins() {
    let mut seen = HashSet::new();
    seen.insert("greet.sh".to_owned());
    assert_eq!(
        classify(&dir("", 0), &file("greet.sh"), &seen, text),
        EntryOutcome::Duplicate
    );
}

#[test]
fn a_duplicate_is_rejected_before_its_extension_is_looked_at() {
    // This is the one place the order is observable from outside: a `.md`
    // whose id is already taken comes back as a duplicate, not as forbidden.
    let mut seen = HashSet::new();
    seen.insert("README.md".to_owned());
    assert_eq!(
        classify(&dir("", 0), &file("README.md"), &seen, text),
        EntryOutcome::Duplicate
    );
}

#[test]
fn a_rejected_file_leaves_its_id_free() {
    // Nothing is inserted into `idsSeen` unless the file became a candidate,
    // so a `.md` does not shadow a script of the same id found later.
    let seen = HashSet::new();
    assert_eq!(
        classify(&dir("", 0), &file("notes.md"), &seen, text),
        EntryOutcome::Forbidden
    );
    assert_eq!(
        classify(&dir("", 0), &file("notes.md"), &seen, binary),
        EntryOutcome::Forbidden
    );
}

#[test]
fn a_binary_is_rejected_after_its_extension_passes() {
    let seen = HashSet::new();
    assert_eq!(
        classify(&dir("", 0), &file("a.out"), &seen, binary),
        EntryOutcome::Binary
    );
}

#[test]
fn a_plain_text_file_becomes_a_candidate_with_its_id() {
    let seen = HashSet::new();
    assert_eq!(
        classify(&dir("tools", 1), &file("greet.sh"), &seen, text),
        EntryOutcome::Candidate("tools.greet.sh".to_owned())
    );
}

#[test]
fn custom_directories_are_searched_before_the_defaults() {
    // Order is what makes a user's own script shadow a packaged one of the
    // same id, because the first id seen wins.
    let custom = vec![PathBuf::from("/home/me/scripts")];
    let defaults = vec![PathBuf::from("/usr/share/scripts")];
    assert_eq!(
        scan_directories(&custom, &defaults),
        vec![
            PathBuf::from("/home/me/scripts"),
            PathBuf::from("/usr/share/scripts")
        ]
    );
}

// --- the command line ---------------------------------------------------

const INLINE: &str = "#!/bin/sh\n\
# @raycast.schemaVersion 1\n\
# @raycast.title Greet\n\
# @raycast.mode inline\n";

const SILENT: &str = "#!/bin/sh\n\
# @raycast.schemaVersion 1\n\
# @raycast.title Greet\n\
# @raycast.mode silent\n";

fn silent_with(extra: &str) -> String {
    format!("{SILENT}{extra}")
}

fn parse(path: &str, text: &str) -> ScriptCommandFile {
    ScriptCommandFile::parse(path, "greet.sh", text).expect("the fixture parses")
}

#[test]
fn the_interpreter_runs_the_script_by_path() {
    let script = parse("/s/tools/greet.sh", SILENT);
    assert_eq!(
        script.command_line(&["/bin/sh".to_owned()], &[]),
        vec!["/bin/sh".to_owned(), "/s/tools/greet.sh".to_owned()]
    );
}

#[test]
fn an_explicit_exec_replaces_the_interpreter_entirely() {
    // The two scopes may not be mixed in one file, so this fixture is
    // @vicinae throughout.
    let script = parse(
        "/s/tools/greet.sh",
        "#!/bin/sh\n\
         # @vicinae.schemaVersion 1\n\
         # @vicinae.title Greet\n\
         # @vicinae.mode silent\n\
         # @vicinae.exec [\"/usr/bin/env\", \"python3\"]\n",
    );
    assert_eq!(
        script.command_line(&["/bin/sh".to_owned()], &[]),
        vec![
            "/usr/bin/env".to_owned(),
            "python3".to_owned(),
            "/s/tools/greet.sh".to_owned()
        ]
    );
}

#[test]
fn declared_arguments_are_passed_in_order() {
    let script = parse(
        "/s/greet.sh",
        &silent_with(
            "# @raycast.argument1 { \"type\": \"text\" }\n\
             # @raycast.argument2 { \"type\": \"text\" }\n",
        ),
    );
    assert_eq!(
        script.command_line(&[], &["one".to_owned(), "two".to_owned()]),
        vec!["/s/greet.sh".to_owned(), "one".to_owned(), "two".to_owned()]
    );
}

#[test]
fn a_value_with_no_declared_argument_is_dropped() {
    // `std::views::zip` stops at the shorter range, so an extra value never
    // reaches the script — it does not become a trailing positional.
    let script = parse(
        "/s/greet.sh",
        &silent_with("# @raycast.argument1 { \"type\": \"text\" }\n"),
    );
    assert_eq!(
        script.command_line(&[], &["one".to_owned(), "two".to_owned()]),
        vec!["/s/greet.sh".to_owned(), "one".to_owned()]
    );
}

#[test]
fn a_declared_argument_with_no_value_is_not_passed_as_empty() {
    let script = parse(
        "/s/greet.sh",
        &silent_with(
            "# @raycast.argument1 { \"type\": \"text\" }\n\
             # @raycast.argument2 { \"type\": \"text\" }\n",
        ),
    );
    assert_eq!(
        script.command_line(&[], &["one".to_owned()]),
        vec!["/s/greet.sh".to_owned(), "one".to_owned()]
    );
}

#[test]
fn a_percent_encoded_argument_is_encoded_and_its_neighbour_is_not() {
    let script = parse(
        "/s/greet.sh",
        &silent_with(
            "# @raycast.argument1 { \"type\": \"text\", \"percentEncoded\": true }\n\
             # @raycast.argument2 { \"type\": \"text\" }\n",
        ),
    );
    assert_eq!(
        script.command_line(&[], &["a b".to_owned(), "a b".to_owned()]),
        vec![
            "/s/greet.sh".to_owned(),
            "a%20b".to_owned(),
            "a b".to_owned()
        ]
    );
}

#[test]
fn percent_encoding_leaves_the_unreserved_set_alone() {
    assert_eq!(percent_encode("aZ09-._~"), "aZ09-._~");
}

#[test]
fn percent_encoding_escapes_slashes_and_ampersands() {
    // Qt's default encodes everything outside the unreserved set, which is the
    // point: the value is going onto a command line, not into a path.
    assert_eq!(percent_encode("a/b&c"), "a%2Fb%26c");
}

#[test]
fn percent_encoding_works_on_bytes_not_characters() {
    assert_eq!(percent_encode("é"), "%C3%A9");
}

// --- the package name ---------------------------------------------------

#[test]
fn a_script_with_no_package_name_is_grouped_by_its_directory() {
    let script = parse("/s/tools/greet.sh", SILENT);
    assert_eq!(script.package_name(None), "tools");
}

#[test]
fn a_declared_package_name_wins_over_the_directory() {
    let script = parse(
        "/s/tools/greet.sh",
        &silent_with("# @raycast.packageName Utilities\n"),
    );
    assert_eq!(script.package_name(None), "Utilities");
}

#[test]
fn an_inline_script_shows_its_output_instead_of_a_group() {
    let script = parse("/s/tools/greet.sh", INLINE);
    assert_eq!(script.package_name(Some("42°C")), "42°C");
}

#[test]
fn an_inline_script_that_has_never_run_says_so() {
    let script = parse("/s/tools/greet.sh", INLINE);
    assert_eq!(script.package_name(None), "No data");
}

#[test]
fn an_inline_scripts_own_package_name_is_ignored() {
    // The output takes the slot the package name would occupy, so declaring
    // one on an inline script has no effect at all.
    let script = parse(
        "/s/tools/greet.sh",
        &format!("{INLINE}# @raycast.packageName Utilities\n"),
    );
    assert_eq!(script.package_name(None), "No data");
    assert_eq!(script.package_name(Some("ok")), "ok");
}

// --- the icon -----------------------------------------------------------

fn never_emoji(_: &str) -> bool {
    false
}

fn nothing_exists(_: &Path) -> bool {
    false
}

#[test]
fn a_script_with_no_icon_gets_the_tinted_code_glyph() {
    let script = parse("/s/greet.sh", SILENT);
    let icon = script.icon(never_emoji, nothing_exists);
    assert_eq!(icon, default_icon());
    assert_eq!(icon.name, "code");
    assert_eq!(
        icon.background_tint,
        Some(ColorLike::Semantic("Accent".into()))
    );
}

#[test]
fn an_emoji_icon_is_taken_before_anything_is_looked_up_on_disk() {
    let script = parse("/s/greet.sh", &silent_with("# @raycast.icon 🎉\n"));
    let icon = script.icon(|s| s == "🎉", |_| panic!("the disk must not be touched"));
    assert_eq!(icon.kind, ImageUrlType::Emoji);
    assert_eq!(icon.name, "🎉");
}

#[test]
fn an_icon_path_that_exists_as_written_is_used() {
    // An absolute path would not distinguish the two branches — joining it
    // onto the script's directory replaces the directory and lands on the same
    // file. A *relative* name that resolves against the process's working
    // directory does: it is taken as written, not rewritten to sit beside the
    // script.
    let script = parse("/s/tools/greet.sh", &silent_with("# @raycast.icon x.png\n"));
    let icon = script.icon(never_emoji, |p| p == Path::new("x.png"));
    assert_eq!(icon.kind, ImageUrlType::Local);
    assert_eq!(icon.name, "x.png");
}

#[test]
fn an_absolute_icon_path_is_never_rewritten_to_sit_beside_the_script() {
    let script = parse(
        "/s/tools/greet.sh",
        &silent_with("# @raycast.icon /opt/x.png\n"),
    );
    let icon = script.icon(never_emoji, |p| p == Path::new("/opt/x.png"));
    assert_eq!(icon.kind, ImageUrlType::Local);
    assert_eq!(icon.name, "/opt/x.png");
}

#[test]
fn an_icon_is_otherwise_looked_for_beside_the_script() {
    // This is what lets a script ship its icon next to it and name it with a
    // bare filename.
    let script = parse("/s/tools/greet.sh", &silent_with("# @raycast.icon x.png\n"));
    let icon = script.icon(never_emoji, |p| p == Path::new("/s/tools/x.png"));
    assert_eq!(icon.kind, ImageUrlType::Local);
    assert_eq!(icon.name, "/s/tools/x.png");
}

#[test]
fn an_https_icon_becomes_a_remote_image() {
    let script = parse(
        "/s/greet.sh",
        &silent_with("# @raycast.icon https://example.com/x.png\n"),
    );
    let icon = script.icon(never_emoji, nothing_exists);
    assert_eq!(icon.kind, ImageUrlType::Http);
    assert_eq!(icon.name, "https://example.com/x.png");
}

#[test]
fn a_plain_http_icon_is_refused() {
    // The C++ tests the scheme for `https` exactly, so an `http` icon falls
    // through to the default rather than being fetched in the clear.
    let script = parse(
        "/s/greet.sh",
        &silent_with("# @raycast.icon http://example.com/x.png\n"),
    );
    assert_eq!(script.icon(never_emoji, nothing_exists), default_icon());
}

#[test]
fn a_missing_icon_file_falls_through_to_the_default() {
    let script = parse("/s/greet.sh", &silent_with("# @raycast.icon x.png\n"));
    assert_eq!(script.icon(never_emoji, nothing_exists), default_icon());
}

#[test]
fn a_scheme_is_lowercased_the_way_qurl_lowercases_it() {
    assert_eq!(url_scheme("HTTPS://example.com").as_deref(), Some("https"));
    assert_eq!(url_scheme("/opt/x.png"), None);
    assert_eq!(url_scheme("9x:y"), None);
}

// --- the metadata store -------------------------------------------------

#[test]
fn an_output_line_survives_a_round_trip() {
    let mut store = ScriptMetadataStore::new();
    store.save_run("weather", "12°C, clear", 1_700_000_000);
    let reloaded = ScriptMetadataStore::from_json(&store.to_json());
    assert_eq!(
        reloaded.last_run_data("weather").as_deref(),
        Some("12°C, clear")
    );
    assert_eq!(reloaded.last_run_at("weather"), Some(1_700_000_000));
}

#[test]
fn the_output_is_stored_base64_encoded() {
    // The C++ comment gives the reason: the line is whatever someone else's
    // script printed, and encoding it keeps the file valid JSON regardless.
    let mut store = ScriptMetadataStore::new();
    store.save_run("weather", "ok", 1);
    let json = store.to_json();
    assert!(json.contains("b2s="), "expected base64 in {json}");
    assert!(!json.contains("\"ok\""));
}

#[test]
fn a_line_with_quotes_and_newlines_round_trips() {
    let mut store = ScriptMetadataStore::new();
    store.save_run("x", "he said \"hi\"\nthen left", 1);
    let reloaded = ScriptMetadataStore::from_json(&store.to_json());
    assert_eq!(
        reloaded.last_run_data("x").as_deref(),
        Some("he said \"hi\"\nthen left")
    );
}

#[test]
fn a_second_run_replaces_the_first() {
    let mut store = ScriptMetadataStore::new();
    store.save_run("x", "old", 1);
    store.save_run("x", "new", 2);
    assert_eq!(store.last_run_data("x").as_deref(), Some("new"));
    assert_eq!(store.last_run_at("x"), Some(2));
}

#[test]
fn a_script_that_never_ran_has_no_output() {
    let store = ScriptMetadataStore::new();
    assert_eq!(store.last_run_data("x"), None);
}

#[test]
fn a_corrupt_metadata_file_leaves_an_empty_store_rather_than_an_error() {
    // Losing the cached output of inline scripts is not a reason to refuse to
    // list anyone's scripts, so the C++ logs and carries on.
    let store = ScriptMetadataStore::from_json("{ not json");
    assert_eq!(store.last_run_data("x"), None);
    assert_eq!(store.to_json(), "{\"inline_runs\":{}}");
}

#[test]
fn the_stored_key_is_snake_case() {
    let mut store = ScriptMetadataStore::new();
    store.save_run("x", "ok", 7);
    let json = store.to_json();
    assert!(json.contains("inline_runs"), "{json}");
    assert!(json.contains("last_run_at"), "{json}");
}
