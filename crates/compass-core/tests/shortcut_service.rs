//! The quicklink list the launcher reads from.
//!
//! Ported from `ShortcutService`.

use std::path::{Path, PathBuf};

use compass_core::shortcut::{DEFAULT_APP_ID, UrlPart};
use compass_core::shortcut_service::{ShortcutService, from_serialized};
use compass_core::shortcut_store::{SerializedShortcut, ShortcutStore};

const NOW: u64 = 1_700_000_000;

/// A directory that goes away with the test.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("compass-shortcut-service-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch directory");
        Self(dir)
    }

    fn file(&self) -> PathBuf {
        self.0.join("shortcuts.json")
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn stored(id: &str, url: &str) -> SerializedShortcut {
    SerializedShortcut {
        id: id.to_owned(),
        name: "Search".to_owned(),
        icon: "globe".to_owned(),
        url: url.to_owned(),
        app: DEFAULT_APP_ID.to_owned(),
        open_count: 3,
        created_at: 10,
        updated_at: 20,
        last_used_at: Some(30),
    }
}

fn write_store(path: &Path, shortcuts: &[SerializedShortcut]) {
    std::fs::write(path, serde_json::to_string(shortcuts).expect("serialize")).expect("write");
}

#[test]
fn a_cached_shortcut_carries_its_link_already_parsed() {
    let cached = from_serialized(&stored("sct-1", "https://x.test/?q={query}"));

    assert_eq!(cached.link.raw, "https://x.test/?q={query}");
    assert_eq!(cached.link.arguments.len(), 1);
    assert_eq!(cached.link.arguments[0].name, "query");
}

#[test]
fn every_stored_field_reaches_the_cached_shortcut() {
    let cached = from_serialized(&stored("sct-1", "https://x.test/"));

    assert_eq!(cached.id, "sct-1");
    assert_eq!(cached.name, "Search");
    assert_eq!(cached.icon, "globe");
    assert_eq!(cached.app, DEFAULT_APP_ID);
    assert_eq!(cached.open_count, 3);
    assert_eq!(cached.created_at, 10);
    assert_eq!(cached.updated_at, 20);
}

#[test]
fn a_quicklink_never_opened_has_no_last_opened_time() {
    let mut never = stored("sct-1", "https://x.test/");
    never.last_used_at = None;

    let cached = from_serialized(&never);

    // Not `Some(0)`: that would date it to the epoch, which reads as opened
    // rather than as never opened.
    assert_eq!(cached.last_opened_at, None);
}

#[test]
fn opening_builds_the_list_in_file_order() {
    let scratch = Scratch::new("order");
    write_store(
        &scratch.file(),
        &[
            stored("sct-b", "https://b.test/"),
            stored("sct-a", "https://a.test/"),
        ],
    );

    let service = ShortcutService::open(&scratch.file());

    let ids: Vec<&str> = service.shortcuts().iter().map(|s| s.id.as_str()).collect();
    assert_eq!(ids, ["sct-b", "sct-a"]);
}

#[test]
fn a_migrated_file_is_visible_to_a_service_opened_after_it() {
    // The constructor's ordering constraint: the migration writes the file and
    // the list is built from the file, so a list built first would be empty.
    let scratch = Scratch::new("after-migration");
    let service = ShortcutService::open(&scratch.file());
    assert!(service.shortcuts().is_empty());

    write_store(&scratch.file(), &[stored("sct-1", "https://x.test/")]);

    let after = ShortcutService::open(&scratch.file());
    assert_eq!(after.shortcuts().len(), 1);
}

#[test]
fn a_shortcut_is_found_by_id() {
    let scratch = Scratch::new("find");
    write_store(&scratch.file(), &[stored("sct-1", "https://x.test/")]);

    let service = ShortcutService::open(&scratch.file());

    assert!(service.find_by_id("sct-1").is_some());
    assert!(service.find_by_id("sct-2").is_none());
}

#[test]
fn creating_appends_to_the_list() {
    let scratch = Scratch::new("create");
    let mut service = ShortcutService::open(&scratch.file());

    assert!(service.create("Docs", "book", "https://docs.test/", DEFAULT_APP_ID, NOW));

    assert_eq!(service.shortcuts().len(), 1);
    assert_eq!(service.shortcuts()[0].name, "Docs");
}

#[test]
fn a_created_shortcut_has_its_link_parsed_too() {
    let scratch = Scratch::new("create-parsed");
    let mut service = ShortcutService::open(&scratch.file());

    assert!(service.create(
        "Docs",
        "book",
        "https://d.test/{query}",
        DEFAULT_APP_ID,
        NOW
    ));

    assert_eq!(service.shortcuts()[0].link.arguments.len(), 1);
}

#[test]
fn a_created_shortcut_is_on_disk() {
    let scratch = Scratch::new("create-disk");
    let mut service = ShortcutService::open(&scratch.file());
    assert!(service.create("Docs", "book", "https://docs.test/", DEFAULT_APP_ID, NOW));

    let reopened = ShortcutService::open(&scratch.file());

    assert_eq!(reopened.shortcuts().len(), 1);
}

#[test]
fn a_created_shortcut_starts_unused() {
    let scratch = Scratch::new("create-unused");
    let mut service = ShortcutService::open(&scratch.file());
    assert!(service.create("Docs", "book", "https://docs.test/", DEFAULT_APP_ID, NOW));

    assert_eq!(service.shortcuts()[0].open_count, 0);
    assert_eq!(service.shortcuts()[0].last_opened_at, None);
}

#[test]
fn updating_changes_the_four_editable_fields() {
    let scratch = Scratch::new("update");
    write_store(
        &scratch.file(),
        &[
            stored("sct-0", "https://first.test/"),
            stored("sct-1", "https://x.test/"),
        ],
    );
    let mut service = ShortcutService::open(&scratch.file());

    assert!(service.update(
        "sct-1",
        "Renamed",
        "star",
        "https://y.test/",
        "firefox",
        NOW
    ));

    // The one that was *not* edited is checked too: with a single shortcut in
    // the store, writing to the first entry and writing to the right entry are
    // the same thing, and a control said so.
    let untouched = service.find_by_id("sct-0").expect("still there");
    assert_eq!(untouched.name, "Search");
    assert_eq!(untouched.link.raw, "https://first.test/");

    let shortcut = service.find_by_id("sct-1").expect("still there");
    assert_eq!(shortcut.name, "Renamed");
    assert_eq!(shortcut.icon, "star");
    assert_eq!(shortcut.link.raw, "https://y.test/");
    assert_eq!(shortcut.app, "firefox");
    assert_eq!(shortcut.updated_at, NOW);
}

#[test]
fn an_updated_link_is_reparsed_rather_than_left_as_it_was() {
    let scratch = Scratch::new("update-parse");
    write_store(&scratch.file(), &[stored("sct-1", "https://x.test/")]);
    let mut service = ShortcutService::open(&scratch.file());
    assert!(
        service
            .find_by_id("sct-1")
            .expect("there")
            .link
            .arguments
            .is_empty()
    );

    assert!(service.update(
        "sct-1",
        "Search",
        "globe",
        "https://x.test/?q={query}",
        DEFAULT_APP_ID,
        NOW
    ));

    let link = &service.find_by_id("sct-1").expect("still there").link;
    assert_eq!(link.arguments.len(), 1);
    assert!(matches!(link.parts.last(), Some(UrlPart::Placeholder(_))));
}

#[test]
fn updating_does_not_reset_how_often_a_quicklink_was_opened() {
    let scratch = Scratch::new("update-count");
    write_store(&scratch.file(), &[stored("sct-1", "https://x.test/")]);
    let mut service = ShortcutService::open(&scratch.file());

    assert!(service.update(
        "sct-1",
        "Renamed",
        "star",
        "https://y.test/",
        "firefox",
        NOW
    ));

    let shortcut = service.find_by_id("sct-1").expect("still there");
    assert_eq!(shortcut.open_count, 3);
    assert_eq!(shortcut.created_at, 10);
    assert_eq!(shortcut.last_opened_at, Some(30));
}

#[test]
fn updating_an_unknown_id_changes_nothing() {
    let scratch = Scratch::new("update-unknown");
    write_store(&scratch.file(), &[stored("sct-1", "https://x.test/")]);
    let mut service = ShortcutService::open(&scratch.file());
    let before = service.shortcuts().to_vec();

    assert!(!service.update(
        "sct-2",
        "Renamed",
        "star",
        "https://y.test/",
        "firefox",
        NOW
    ));

    assert_eq!(service.shortcuts(), before.as_slice());
}

#[test]
fn an_update_the_file_refused_leaves_the_list_alone() {
    // The file is replaced by a directory, so the write fails and nothing else
    // about the service changes.
    let scratch = Scratch::new("update-unwritable");
    write_store(&scratch.file(), &[stored("sct-1", "https://x.test/")]);
    let mut service = ShortcutService::open(&scratch.file());
    std::fs::remove_file(scratch.file()).expect("remove");
    std::fs::create_dir(scratch.file()).expect("directory in its place");

    assert!(!service.update(
        "sct-1",
        "Renamed",
        "star",
        "https://y.test/",
        "firefox",
        NOW
    ));

    let shortcut = service.find_by_id("sct-1").expect("still there");
    assert_eq!(shortcut.name, "Search");
    assert_eq!(shortcut.link.raw, "https://x.test/");
}

#[test]
fn a_creation_the_file_refused_leaves_the_list_alone() {
    let scratch = Scratch::new("create-unwritable");
    let mut service = ShortcutService::open(&scratch.file());
    std::fs::remove_file(scratch.file()).expect("remove");
    std::fs::create_dir(scratch.file()).expect("directory in its place");

    assert!(!service.create("Docs", "book", "https://docs.test/", DEFAULT_APP_ID, NOW));

    assert!(service.shortcuts().is_empty());
}

#[test]
fn removing_takes_it_out_of_the_list() {
    let scratch = Scratch::new("remove");
    write_store(
        &scratch.file(),
        &[
            stored("sct-1", "https://x.test/"),
            stored("sct-2", "https://y.test/"),
        ],
    );
    let mut service = ShortcutService::open(&scratch.file());

    assert!(service.remove("sct-1"));

    let ids: Vec<&str> = service.shortcuts().iter().map(|s| s.id.as_str()).collect();
    assert_eq!(ids, ["sct-2"]);
}

#[test]
fn removing_an_unknown_id_changes_nothing() {
    let scratch = Scratch::new("remove-unknown");
    write_store(&scratch.file(), &[stored("sct-1", "https://x.test/")]);
    let mut service = ShortcutService::open(&scratch.file());

    assert!(!service.remove("sct-2"));

    assert_eq!(service.shortcuts().len(), 1);
}

#[test]
fn a_visit_counts_and_dates_the_quicklink() {
    let scratch = Scratch::new("visit");
    let mut first = stored("sct-0", "https://first.test/");
    first.open_count = 99;
    write_store(
        &scratch.file(),
        &[first, stored("sct-1", "https://x.test/")],
    );
    let mut service = ShortcutService::open(&scratch.file());

    assert!(service.register_visit("sct-1", NOW));

    let shortcut = service.find_by_id("sct-1").expect("still there");
    assert_eq!(shortcut.open_count, 4);
    assert_eq!(shortcut.last_opened_at, Some(NOW));

    // Reading the count back from the first entry rather than the visited one
    // would put 100 here, and with one shortcut in the store nothing could
    // tell the two apart.
    let untouched = service.find_by_id("sct-0").expect("still there");
    assert_eq!(untouched.open_count, 99);
    assert_eq!(untouched.last_opened_at, Some(30));
}

#[test]
fn a_visit_takes_the_count_the_file_now_holds() {
    let scratch = Scratch::new("visit-from-file");
    write_store(&scratch.file(), &[stored("sct-1", "https://x.test/")]);
    let mut service = ShortcutService::open(&scratch.file());
    assert!(service.register_visit("sct-1", NOW));

    let on_disk = ShortcutStore::open(&scratch.file());

    assert_eq!(
        service.find_by_id("sct-1").expect("there").open_count,
        on_disk.find_by_id("sct-1").expect("there").open_count
    );
}

#[test]
fn a_visit_to_an_unknown_id_changes_nothing() {
    let scratch = Scratch::new("visit-unknown");
    write_store(&scratch.file(), &[stored("sct-1", "https://x.test/")]);
    let mut service = ShortcutService::open(&scratch.file());
    let before = service.shortcuts().to_vec();

    assert!(!service.register_visit("sct-2", NOW));

    assert_eq!(service.shortcuts(), before.as_slice());
}

#[test]
fn a_visit_the_file_refused_leaves_the_count_alone() {
    let scratch = Scratch::new("visit-unwritable");
    write_store(&scratch.file(), &[stored("sct-1", "https://x.test/")]);
    let mut service = ShortcutService::open(&scratch.file());
    std::fs::remove_file(scratch.file()).expect("remove");
    std::fs::create_dir(scratch.file()).expect("directory in its place");

    assert!(!service.register_visit("sct-1", NOW));

    assert_eq!(service.find_by_id("sct-1").expect("there").open_count, 3);
}

#[test]
fn reloading_takes_the_file_over_the_list() {
    let scratch = Scratch::new("reload");
    write_store(&scratch.file(), &[stored("sct-1", "https://x.test/")]);
    let mut service = ShortcutService::open(&scratch.file());

    write_store(
        &scratch.file(),
        &[
            stored("sct-1", "https://x.test/"),
            stored("sct-9", "https://z.test/"),
        ],
    );
    service.reload();

    let ids: Vec<&str> = service.shortcuts().iter().map(|s| s.id.as_str()).collect();
    assert_eq!(ids, ["sct-1", "sct-9"]);
}
