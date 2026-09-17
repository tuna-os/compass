//! The write path, against real encrypted databases.
//!
//! Two of these are about bugs fixed rather than ported, so they assert on
//! behaviour the C++ engine does not have.

use std::time::Duration;

use compass_clipboard::kind::{EncryptionType, OfferKind};
use compass_clipboard::schema;
use compass_clipboard::store::{self, ListSettings};
use compass_clipboard::write::{self, NewOffer, NewSelection};
use compass_sqlcipher_sys::Database;

const KEY: &[u8] = &[0x44; 32];

struct Db {
    _dir: tempfile::TempDir,
    db: Database,
}

fn fresh() -> Db {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let db = Database::open(&dir.path().join("clip.db"), KEY).expect("open");
    schema::run(&db).expect("migrations");
    Db { _dir: dir, db }
}

fn add(db: &Database, id: &str, text: &str) {
    write::insert_selection(
        db,
        &NewSelection {
            id,
            offer_count: 1,
            hash: &format!("hash-{id}"),
            preferred_mime_type: "text/plain",
            kind: OfferKind::Text,
            source: Some("test"),
        },
    )
    .expect("insert selection");
    write::insert_offer(
        db,
        &NewOffer {
            id: &format!("offer-{id}"),
            selection_id: id,
            mime_type: "text/plain",
            text_preview: text,
            md5sum: &format!("hash-{id}"),
            encryption: EncryptionType::None,
            kind: OfferKind::Text,
            size: text.len() as i64,
            url_host: None,
        },
    )
    .expect("insert offer");
    write::index_content(db, id, text).expect("index");
}

fn age(db: &Database, id: &str, seconds_ago: i64) {
    let mut stmt = db
        .prepare("UPDATE selection SET updated_at = unixepoch() - :ago WHERE id = :id")
        .expect("prepare");
    stmt.bind_int64(":ago", seconds_ago).expect("bind");
    stmt.bind_text(":id", id).expect("bind");
    stmt.step().expect("age it");
}

#[test]
fn an_inserted_selection_is_readable_through_the_query() {
    let d = fresh();
    add(&d.db, "one", "hello world");

    let page = store::query(&d.db, 10, 0, &ListSettings::default()).expect("query");
    assert_eq!(page.data.len(), 1);
    assert_eq!(page.data[0].id, "one");
    assert_eq!(page.data[0].text_preview, "hello world");

    // And it is searchable, which is index_content's job rather than the
    // insert's.
    let hits = store::query(
        &d.db,
        10,
        0,
        &ListSettings {
            query: "wor".to_owned(),
            kind: None,
        },
    )
    .expect("query");
    assert_eq!(hits.data.len(), 1);
}

#[test]
fn a_nullable_column_left_unset_reads_back_as_null() {
    let d = fresh();
    add(&d.db, "one", "text");
    let page = store::query(&d.db, 10, 0, &ListSettings::default()).expect("query");
    assert_eq!(page.data[0].url_host, None);
}

fn updated_at(db: &Database, id: &str) -> i64 {
    let mut stmt = db
        .prepare("SELECT updated_at FROM selection WHERE id = :id")
        .expect("prepare");
    stmt.bind_text(":id", id).expect("bind");
    assert!(stmt.step().expect("step"), "no such selection: {id}");
    stmt.column_int64(0)
}

#[test]
fn bubbling_up_moves_a_selection_to_now() {
    // Asserted on updated_at rather than on list order, deliberately.
    // `updated_at` is whole seconds -- `QDateTime::currentSecsSinceEpoch()` in
    // the C++, and the same here -- so two selections bubbled within the same
    // second tie, and `ORDER BY updated_at DESC` leaves their relative order to
    // SQLite. An earlier version of this test asserted the order and failed on
    // exactly that tie. The tie is a real property of second granularity, not
    // something either engine promises, so the test does not lean on it.
    let d = fresh();
    add(&d.db, "a", "first");
    add(&d.db, "b", "second");
    age(&d.db, "a", 500);
    age(&d.db, "b", 500);

    let before = updated_at(&d.db, "a");

    // By id.
    assert!(write::bubble_up(&d.db, "a").expect("bubble"));
    assert!(
        updated_at(&d.db, "a") > before,
        "bubbling must move updated_at forward"
    );
    assert_eq!(
        updated_at(&d.db, "b"),
        before,
        "and must not touch anything else"
    );

    // By content hash, which is the path clipboard-service.cpp actually uses.
    assert!(write::bubble_up(&d.db, "hash-b").expect("bubble"));
    assert!(updated_at(&d.db, "b") > before);
}

#[test]
fn a_bubbled_selection_sorts_above_an_older_one() {
    // The ordering claim, made where the timestamps genuinely differ.
    let d = fresh();
    add(&d.db, "a", "first");
    add(&d.db, "b", "second");
    age(&d.db, "a", 500);
    age(&d.db, "b", 900);

    assert!(write::bubble_up(&d.db, "b").expect("bubble"));
    let page = store::query(&d.db, 10, 0, &ListSettings::default()).expect("query");
    assert_eq!(
        page.data[0].id, "b",
        "bubbling to now puts it above a selection 500 seconds old"
    );
}

#[test]
fn bubbling_up_something_absent_reports_false_even_after_a_successful_write() {
    // The bug this replaces. tryBubbleUpSelection answers from the
    // connection-wide sqlite3_changes counter, which still holds the count from
    // the previous successful write -- so after a write that changed rows, a
    // bubble-up of something absent could report success. Its caller then skips
    // insertSelection and the copied content never reaches the history.
    let d = fresh();
    add(&d.db, "a", "first");
    assert!(
        write::bubble_up(&d.db, "a").expect("bubble"),
        "precondition: a real bubble-up reports true, so the next assertion is \
         not passing merely because the function always returns false"
    );

    assert!(
        !write::bubble_up(&d.db, "no-such-selection").expect("bubble"),
        "an absent selection must report false, whatever the last statement changed"
    );
}

#[test]
fn removing_a_selection_returns_the_offers_to_unlink() {
    let d = fresh();
    add(&d.db, "one", "text");

    let removed = write::remove_selection(&d.db, "one").expect("remove");
    assert_eq!(removed, vec!["offer-one"]);

    let page = store::query(&d.db, 10, 0, &ListSettings::default()).expect("query");
    assert!(page.data.is_empty());
}

#[test]
fn eviction_returns_every_offer_it_deletes() {
    // The leak this replaces: the C++ computes its cutoff with unixepoch() in
    // BOTH the SELECT that collects offer ids and the DELETE that removes the
    // rows. Those are separate statements with separate readings of the clock,
    // so the DELETE set is a superset and the difference is deleted-but-never-
    // reported: payloads unreferenced on disk forever.
    //
    // Binding one cutoff to both makes the two sets identical by construction.
    // This asserts the property that guarantees: nothing is deleted whose offer
    // id was not returned.
    let d = fresh();
    for i in 0..6 {
        let id = format!("s{i}");
        add(&d.db, &id, "text");
        age(&d.db, &id, i64::from(i) * 100);
    }

    let evicted = write::evict_older_than(&d.db, Duration::from_secs(250), true).expect("evict");

    let remaining = store::query(&d.db, 100, 0, &ListSettings::default()).expect("query");
    let remaining_ids: Vec<&str> = remaining.data.iter().map(|e| e.id.as_str()).collect();

    // s3, s4, s5 are 300, 400 and 500 seconds old.
    assert_eq!(evicted.len(), 3, "got {evicted:?}");
    for id in ["s3", "s4", "s5"] {
        assert!(
            evicted.contains(&format!("offer-{id}")),
            "{id} was evicted, so its offer must be reported"
        );
        assert!(!remaining_ids.contains(&id), "{id} should be gone");
    }

    // The property, stated directly: every row that disappeared was reported.
    let mut stmt =
        d.db.prepare("SELECT count(*) FROM data_offer")
            .expect("prepare");
    assert!(stmt.step().expect("step"));
    assert_eq!(
        stmt.column_int64(0),
        3,
        "three offers remain, so exactly the three reported were deleted"
    );
}

#[test]
fn eviction_preserves_pinned_and_tagged_entries() {
    let d = fresh();
    add(&d.db, "old", "text");
    age(&d.db, "old", 1000);
    add(&d.db, "pinned", "text");
    age(&d.db, "pinned", 1000);
    add(&d.db, "tagged", "text");
    age(&d.db, "tagged", 1000);

    let mut stmt =
        d.db.prepare("UPDATE selection SET pinned_at = 1 WHERE id = 'pinned'")
            .expect("prepare");
    stmt.step().expect("pin");
    drop(stmt);
    let mut stmt =
        d.db.prepare("UPDATE selection SET keywords = 'keep' WHERE id = 'tagged'")
            .expect("prepare");
    stmt.step().expect("tag");
    drop(stmt);

    let evicted = write::evict_older_than(&d.db, Duration::from_secs(10), true).expect("evict");
    assert_eq!(evicted, vec!["offer-old"]);

    // And without preserve_tagged, all three go.
    let evicted = write::evict_older_than(&d.db, Duration::from_secs(10), false).expect("evict");
    assert_eq!(evicted.len(), 2, "got {evicted:?}");
}

#[test]
fn eviction_of_nothing_returns_nothing_and_deletes_nothing() {
    let d = fresh();
    add(&d.db, "recent", "text");

    let evicted = write::evict_older_than(&d.db, Duration::from_secs(3600), true).expect("evict");
    assert!(evicted.is_empty());

    let page = store::query(&d.db, 10, 0, &ListSettings::default()).expect("query");
    assert_eq!(page.data.len(), 1, "a no-op eviction must not delete");
}

#[test]
fn remove_all_can_spare_the_tagged() {
    let d = fresh();
    add(&d.db, "plain", "text");
    add(&d.db, "pinned", "text");
    let mut stmt =
        d.db.prepare("UPDATE selection SET pinned_at = 1 WHERE id = 'pinned'")
            .expect("prepare");
    stmt.step().expect("pin");
    drop(stmt);

    let removed = write::remove_all(&d.db, true).expect("remove all");
    assert_eq!(removed, vec!["offer-plain"]);

    let page = store::query(&d.db, 10, 0, &ListSettings::default()).expect("query");
    assert_eq!(page.data.len(), 1);
    assert_eq!(page.data[0].id, "pinned");

    let removed = write::remove_all(&d.db, false).expect("remove all");
    assert_eq!(removed, vec!["offer-pinned"]);
    let page = store::query(&d.db, 10, 0, &ListSettings::default()).expect("query");
    assert!(page.data.is_empty());
}

#[test]
fn a_duplicate_id_is_refused_rather_than_silently_ignored() {
    let d = fresh();
    add(&d.db, "one", "text");
    let err = write::insert_selection(
        &d.db,
        &NewSelection {
            id: "one",
            offer_count: 1,
            hash: "hash-one",
            preferred_mime_type: "text/plain",
            kind: OfferKind::Text,
            source: None,
        },
    );
    assert!(err.is_err(), "the primary key must reject a duplicate");
}
