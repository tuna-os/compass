//! The quicklink store, read against
//! `src/server/src/services/shortcut/shortcut-db.cpp`.

use compass_core::shortcut_store::{
    Error, ID_LENGTH, ID_PREFIX, MAX_SHORTCUTS, SerializedShortcut, ShortcutStore, generate_id,
};

const NOW: u64 = 1_700_000_000;

fn store(dir: &std::path::Path) -> ShortcutStore {
    ShortcutStore::open(&dir.join("shortcuts/shortcuts.json"))
}

fn file(dir: &std::path::Path) -> String {
    std::fs::read_to_string(dir.join("shortcuts/shortcuts.json")).expect("the file exists")
}

#[test]
fn opening_a_missing_store_creates_the_directory_and_an_empty_array() {
    // `fs::create_directories(path.parent_path())` then `setShortcuts({})`.
    let dir = tempfile::tempdir().expect("tempdir");
    let shortcuts = store(dir.path());

    assert!(shortcuts.shortcuts().is_empty());
    assert_eq!(file(dir.path()), "[]");
}

#[test]
fn a_corrupt_file_reads_as_no_shortcuts_rather_than_failing_to_start() {
    // `loadShortcuts().value_or(std::vector<SerializedShortcut>{})` -- the
    // parse error is dropped. Reproduced: a launcher that would not start
    // because one file was truncated is worse, and this is what users' files
    // have already been through.
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(dir.path().join("shortcuts")).expect("mkdir");
    std::fs::write(dir.path().join("shortcuts/shortcuts.json"), "{not json")
        .expect("write a corrupt file");

    let shortcuts = store(dir.path());
    assert!(shortcuts.shortcuts().is_empty());
    assert_eq!(
        file(dir.path()),
        "{not json",
        "and the corrupt file is left alone until something writes"
    );
}

#[test]
fn adding_stamps_both_timestamps_and_starts_the_counter_at_zero() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut shortcuts = store(dir.path());

    let added = shortcuts
        .add(
            "Search",
            "globe",
            "https://e.org/?q={query}",
            "default",
            NOW,
        )
        .expect("added");

    assert_eq!(
        added,
        SerializedShortcut {
            id: added.id.clone(),
            name: "Search".to_owned(),
            icon: "globe".to_owned(),
            url: "https://e.org/?q={query}".to_owned(),
            app: "default".to_owned(),
            open_count: 0,
            created_at: NOW,
            updated_at: NOW,
            last_used_at: None,
        }
    );
    assert_eq!(shortcuts.shortcuts().len(), 1);
}

#[test]
fn an_id_is_the_prefix_a_dash_and_twelve_hex_characters() {
    // `generatePrefixedId("sct")` with its default length of 12.
    let id = generate_id();
    let (prefix, hex) = id.split_once('-').expect("a dash");

    assert_eq!(prefix, ID_PREFIX);
    assert_eq!(prefix, "sct", "the literal the C++ passes");
    assert_eq!(hex.len(), ID_LENGTH);
    assert_eq!(ID_LENGTH, 12, "generatePrefixedId's default length");
    assert!(
        hex.chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_uppercase()),
        "lower-case hex only: {id}"
    );
    assert_ne!(generate_id(), generate_id(), "and they differ");
}

#[test]
fn the_file_keys_are_the_ones_the_cpp_writes() {
    // The file is written by whichever engine ran last, so the key spelling is
    // a wire format: camelCase, and `lastUsedAt` absent rather than null.
    let dir = tempfile::tempdir().expect("tempdir");
    let mut shortcuts = store(dir.path());
    shortcuts
        .add_with_id(
            "sct-0123456789ab",
            "Search",
            "globe",
            "https://e.org",
            "default",
            NOW,
        )
        .expect("added");

    let written: serde_json::Value = serde_json::from_str(&file(dir.path())).expect("JSON");
    assert_eq!(
        written,
        serde_json::json!([{
            "id": "sct-0123456789ab",
            "name": "Search",
            "icon": "globe",
            "url": "https://e.org",
            "app": "default",
            "openCount": 0,
            "createdAt": NOW,
            "updatedAt": NOW,
        }])
    );
}

#[test]
fn a_store_reads_back_what_the_last_one_wrote() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut first = store(dir.path());
    first
        .add_with_id(
            "sct-aaaaaaaaaaaa",
            "A",
            "a",
            "https://a.org",
            "default",
            NOW,
        )
        .expect("added");

    let second = store(dir.path());
    assert_eq!(second.shortcuts().len(), 1);
    assert_eq!(
        second
            .find_by_id("sct-aaaaaaaaaaaa")
            .map(|s| s.name.as_str()),
        Some("A")
    );
}

#[test]
fn updating_changes_four_fields_and_the_timestamp_and_nothing_else() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut shortcuts = store(dir.path());
    shortcuts
        .add_with_id(
            "sct-aaaaaaaaaaaa",
            "A",
            "a",
            "https://a.org",
            "default",
            NOW,
        )
        .expect("added");
    shortcuts
        .register_visit("sct-aaaaaaaaaaaa", NOW + 10)
        .expect("visited");

    shortcuts
        .update(
            "sct-aaaaaaaaaaaa",
            "B",
            "b",
            "https://b.org",
            "firefox.desktop",
            NOW + 20,
        )
        .expect("updated");

    let shortcut = shortcuts
        .find_by_id("sct-aaaaaaaaaaaa")
        .expect("still there");
    assert_eq!(
        (
            shortcut.name.as_str(),
            shortcut.icon.as_str(),
            shortcut.url.as_str(),
            shortcut.app.as_str()
        ),
        ("B", "b", "https://b.org", "firefox.desktop")
    );
    assert_eq!(shortcut.updated_at, NOW + 20);
    assert_eq!(shortcut.created_at, NOW, "creation is not touched");
    assert_eq!(shortcut.open_count, 1, "nor the visit count");
    assert_eq!(shortcut.last_used_at, Some(NOW + 10), "nor the last visit");
}

#[test]
fn registering_a_visit_counts_it_and_stamps_the_time() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut shortcuts = store(dir.path());
    shortcuts
        .add_with_id(
            "sct-aaaaaaaaaaaa",
            "A",
            "a",
            "https://a.org",
            "default",
            NOW,
        )
        .expect("added");

    shortcuts
        .register_visit("sct-aaaaaaaaaaaa", NOW + 5)
        .expect("visited");
    shortcuts
        .register_visit("sct-aaaaaaaaaaaa", NOW + 9)
        .expect("visited again");

    let shortcut = shortcuts.find_by_id("sct-aaaaaaaaaaaa").expect("there");
    assert_eq!(shortcut.open_count, 2);
    assert_eq!(shortcut.last_used_at, Some(NOW + 9));
}

#[test]
fn removing_answers_with_what_it_removed() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut shortcuts = store(dir.path());
    shortcuts
        .add_with_id(
            "sct-aaaaaaaaaaaa",
            "A",
            "a",
            "https://a.org",
            "default",
            NOW,
        )
        .expect("added");

    let removed = shortcuts.remove("sct-aaaaaaaaaaaa").expect("removed");
    assert_eq!(removed.name, "A");
    assert!(shortcuts.shortcuts().is_empty());
    assert_eq!(file(dir.path()), "[]");
}

#[test]
fn the_two_not_found_messages_are_the_two_the_cpp_uses() {
    // `updateShortcut` and `registerVisit` say "No shortcut with that ID";
    // `removeShortcut` says "No such shortcut". Two sentences for one
    // situation, kept because a log or a UI may already match on them.
    let dir = tempfile::tempdir().expect("tempdir");
    let mut shortcuts = store(dir.path());

    assert_eq!(
        shortcuts.update("sct-nope", "A", "a", "https://a.org", "default", NOW),
        Err(Error::NoSuchId)
    );
    assert_eq!(
        shortcuts.register_visit("sct-nope", NOW),
        Err(Error::NoSuchId)
    );
    assert_eq!(shortcuts.remove("sct-nope"), Err(Error::NoSuchShortcut));

    assert_eq!(Error::NoSuchId.to_string(), "No shortcut with that ID");
    assert_eq!(Error::NoSuchShortcut.to_string(), "No such shortcut");
}

#[test]
fn the_limit_is_ten_thousand_and_its_message_names_it() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut shortcuts = store(dir.path());

    // Filling it one add at a time would rewrite the file 10,000 times; the
    // limit is a property of the list, so seed the file and open it.
    let seeded: Vec<serde_json::Value> = (0..MAX_SHORTCUTS)
        .map(|i| {
            serde_json::json!({
                "id": format!("sct-{i:012x}"), "name": "A", "icon": "a",
                "url": "https://a.org", "app": "default",
                "openCount": 0, "createdAt": NOW, "updatedAt": NOW,
            })
        })
        .collect();
    std::fs::write(
        dir.path().join("shortcuts/shortcuts.json"),
        serde_json::to_string(&seeded).expect("JSON"),
    )
    .expect("seed the file");
    shortcuts.reload();

    assert_eq!(shortcuts.shortcuts().len(), MAX_SHORTCUTS);
    assert_eq!(
        shortcuts.add("One too many", "a", "https://a.org", "default", NOW),
        Err(Error::LimitReached)
    );
    assert_eq!(
        Error::LimitReached.to_string(),
        "Shortcut limit reached (10000)"
    );
}

#[test]
fn a_failed_write_takes_the_new_shortcut_back_out_of_the_list() {
    // `m_shortcuts.pop_back()` on a failed `setShortcuts` -- the list in memory
    // must not claim something the file does not have.
    let dir = tempfile::tempdir().expect("tempdir");
    let mut shortcuts = store(dir.path());

    // A directory where the file should be: the write fails, the read at open
    // time did not.
    let path = dir.path().join("shortcuts/shortcuts.json");
    std::fs::remove_file(&path).expect("remove the file");
    std::fs::create_dir(&path).expect("put a directory in its place");

    let result = shortcuts.add("Search", "globe", "https://e.org", "default", NOW);
    assert!(matches!(result, Err(Error::Write(_))), "{result:?}");
    assert!(
        shortcuts.shortcuts().is_empty(),
        "the entry must not survive the failed write"
    );
}

#[test]
fn reloading_takes_the_file_over_memory() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut shortcuts = store(dir.path());
    shortcuts
        .add_with_id(
            "sct-aaaaaaaaaaaa",
            "A",
            "a",
            "https://a.org",
            "default",
            NOW,
        )
        .expect("added");

    std::fs::write(dir.path().join("shortcuts/shortcuts.json"), "[]").expect("empty the file");
    assert_eq!(shortcuts.shortcuts().len(), 1, "memory still has it");

    shortcuts.reload();
    assert!(shortcuts.shortcuts().is_empty(), "the file wins");
}
