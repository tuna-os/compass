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

// --- migrating from the old SQLite table --------------------------------
//
// Ported from `ShortcutService::migrateFromDatabase` and `resolveApp`.

mod migration {
    use compass_core::shortcut_store::{
        LegacyRow, Migration, ShortcutStore, resolve_app, should_migrate,
    };

    fn row(id: &str) -> LegacyRow {
        LegacyRow {
            id: id.to_owned(),
            name: "Search".to_owned(),
            icon: "link".to_owned(),
            url: "https://example.com/?q={argument}".to_owned(),
            app: "default".to_owned(),
            open_count: 3,
            created_at: 1_700_000_000,
            updated_at: 1_700_000_100,
            last_used_at: Some(1_700_000_200),
        }
    }

    fn store() -> (tempfile::TempDir, ShortcutStore) {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = ShortcutStore::open(&dir.path().join("shortcuts.json"));
        (dir, store)
    }

    #[test]
    fn a_missing_table_is_not_an_error() {
        // It is every installation that never ran the old version.
        let (_dir, mut store) = store();
        assert_eq!(
            store.migrate_from_legacy(false, vec![row("a")]),
            Migration::NoTable
        );
        assert!(store.shortcuts().is_empty());
    }

    #[test]
    fn an_empty_table_writes_nothing_at_all() {
        // Not merely "writes an empty array": an empty write would still
        // create the JSON file, and the next start would then see a store
        // that exists and skip the migration for good.
        let (dir, mut store) = store();
        let path = dir.path().join("shortcuts.json");
        let before = std::fs::read_to_string(&path).ok();

        assert_eq!(
            store.migrate_from_legacy(true, Vec::new()),
            Migration::NoRows
        );
        assert_eq!(std::fs::read_to_string(&path).ok(), before);
    }

    #[test]
    fn rows_are_moved_across_with_every_column() {
        let (_dir, mut store) = store();
        assert_eq!(
            store.migrate_from_legacy(true, vec![row("a")]),
            Migration::Migrated(1)
        );

        let moved = store.find_by_id("a").expect("the row moved");
        assert_eq!(moved.name, "Search");
        assert_eq!(moved.icon, "link");
        assert_eq!(moved.url, "https://example.com/?q={argument}");
        assert_eq!(moved.app, "default");
        assert_eq!(moved.open_count, 3);
        assert_eq!(moved.created_at, 1_700_000_000);
        assert_eq!(moved.updated_at, 1_700_000_100);
        assert_eq!(moved.last_used_at, Some(1_700_000_200));
    }

    #[test]
    fn the_one_nullable_column_survives_being_null() {
        // `last_used_at` is the only nullable column, and a shortcut nobody
        // has opened yet has it unset. Turning that into 0 would make it look
        // used at the epoch.
        let (_dir, mut store) = store();
        let mut never_used = row("a");
        never_used.last_used_at = None;
        store.migrate_from_legacy(true, vec![never_used]);
        assert_eq!(store.find_by_id("a").expect("moved").last_used_at, None);
    }

    #[test]
    fn several_rows_keep_their_order() {
        let (_dir, mut store) = store();
        store.migrate_from_legacy(true, vec![row("a"), row("b"), row("c")]);
        let ids: Vec<&str> = store.shortcuts().iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, ["a", "b", "c"]);
    }

    #[test]
    fn the_migration_survives_a_round_trip_to_disk() {
        // The reload after the write is what proves the JSON is readable
        // back, rather than the store simply holding what it was handed.
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("shortcuts.json");
        let mut store = ShortcutStore::open(&path);
        store.migrate_from_legacy(true, vec![row("a")]);

        let reopened = ShortcutStore::open(&path);
        assert_eq!(reopened.shortcuts().len(), 1);
        assert_eq!(reopened.shortcuts()[0].id, "a");
    }

    #[test]
    fn a_failed_write_leaves_the_store_as_it_was() {
        // Reloading after a failed write is how a migration turns a
        // recoverable problem into an empty shortcut list.
        let dir = tempfile::tempdir().expect("tempdir");
        // A directory where the file should be: the write fails, the read does
        // not error in a way that matters, and nothing is lost.
        let path = dir.path().join("shortcuts.json");
        std::fs::create_dir(&path).expect("a directory in the file's place");
        let mut store = ShortcutStore::open(&path);

        assert_eq!(
            store.migrate_from_legacy(true, vec![row("a")]),
            Migration::Failed
        );

        // What matters is the *file*: nothing was written, so the next start
        // finds an empty store and tries the migration again. The in-memory
        // list is left holding the unwritten rows, as the C++ leaves it — and
        // that is not observable to anyone, because a failed migration is
        // reported and the process does not go on to use the list.
        let reopened = ShortcutStore::open(&path);
        assert!(reopened.shortcuts().is_empty(), "nothing reached the disk");
    }

    #[test]
    fn a_migration_runs_only_into_an_empty_store() {
        // A store with anything in it has been migrated already or has been
        // used since; re-running would duplicate every shortcut or overwrite
        // work done after the move.
        let (_dir, mut store) = store();
        assert!(should_migrate(&store, true));

        store.migrate_from_legacy(true, vec![row("a")]);
        assert!(!should_migrate(&store, true));
    }

    #[test]
    fn no_table_means_no_migration_however_empty_the_store() {
        let (_dir, store) = store();
        assert!(!should_migrate(&store, false));
    }

    // --- which application opens a shortcut -----------------------------

    fn none(_: &str) -> Option<String> {
        None
    }

    #[test]
    fn a_shortcut_naming_an_application_uses_it() {
        let chosen = resolve_app(
            "firefox.desktop",
            "https://example.com",
            |id| Some(id.to_owned()),
            |_| Some("wrong.desktop".to_owned()),
            || Some("browser.desktop".to_owned()),
        );
        assert_eq!(chosen.as_deref(), Some("firefox.desktop"));
    }

    #[test]
    fn a_named_application_that_is_gone_resolves_to_nothing() {
        // Better than silently opening something else: the caller reports it.
        let chosen = resolve_app(
            "uninstalled.desktop",
            "https://example.com",
            none,
            |_| Some("wrong.desktop".to_owned()),
            || Some("browser.desktop".to_owned()),
        );
        assert_eq!(chosen, None);
    }

    #[test]
    fn the_default_uses_whatever_opens_that_target() {
        // Which for a `mailto:` is the mail client, not the browser.
        let chosen = resolve_app(
            "default",
            "mailto:someone@example.com",
            none,
            |target| {
                target
                    .starts_with("mailto:")
                    .then(|| "mail.desktop".to_owned())
            },
            || Some("browser.desktop".to_owned()),
        );
        assert_eq!(chosen.as_deref(), Some("mail.desktop"));
    }

    #[test]
    fn the_browser_is_the_last_resort_and_not_the_rule() {
        // Right for a quicklink and wrong for anything else, so it only
        // applies once the target's own opener has found nothing.
        let chosen = resolve_app("default", "mailto:someone@example.com", none, none, || {
            Some("browser.desktop".to_owned())
        });
        assert_eq!(chosen.as_deref(), Some("browser.desktop"));
    }

    #[test]
    fn nothing_anywhere_resolves_to_nothing() {
        assert_eq!(
            resolve_app("default", "https://example.com", none, none, || None),
            None
        );
    }
}
