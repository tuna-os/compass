//! The paginated query, against real encrypted databases.
//!
//! Seeded through the migrated schema rather than through a hand-written one,
//! so that a query written against columns the migrations do not produce fails
//! here rather than in front of a user.

use compass_clipboard::kind::{EncryptionType, OfferKind};
use compass_clipboard::schema;
use compass_clipboard::store::{self, Error, ListSettings};
use compass_sqlcipher_sys::Database;

const KEY: &[u8] = &[0x33; 32];

struct Seeded {
    _dir: tempfile::TempDir,
    db: Database,
}

/// A database with the schema applied and nothing in it.
fn empty() -> Seeded {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let db = Database::open(&dir.path().join("clip.db"), KEY).expect("open");
    schema::run(&db).expect("migrations");
    Seeded { _dir: dir, db }
}

/// Insert one selection with one offer, and index `content` for search.
#[allow(clippy::too_many_arguments)]
fn insert(
    db: &Database,
    id: &str,
    preview: &str,
    content: &str,
    kind: OfferKind,
    updated_at: i64,
    pinned_at: Option<i64>,
    keywords: &str,
) {
    let mut stmt = db
        .prepare(
            "INSERT INTO selection (id, hash_md5, preferred_mime_type, offer_count, created_at, \
             updated_at, pinned_at, kind, keywords) \
             VALUES (:id, :hash, 'text/plain', 1, :t, :t, :pinned, :kind, :kw)",
        )
        .expect("prepare");
    stmt.bind_text(":id", id).expect("bind");
    stmt.bind_text(":hash", &format!("hash-{id}"))
        .expect("bind");
    stmt.bind_int64(":t", updated_at).expect("bind");
    stmt.bind_int64(":pinned", pinned_at.unwrap_or(0))
        .expect("bind");
    stmt.bind_int64(":kind", kind.to_stored()).expect("bind");
    stmt.bind_text(":kw", keywords).expect("bind");
    stmt.step().expect("insert selection");

    let mut stmt = db
        .prepare(
            "INSERT INTO data_offer (id, selection_id, mime_type, text_preview, \
             content_hash_md5, size, encryption_type, kind, url_host) \
             VALUES (:oid, :id, 'text/plain', :preview, :hash, :size, 0, :kind, NULL)",
        )
        .expect("prepare");
    stmt.bind_text(":oid", &format!("offer-{id}"))
        .expect("bind");
    stmt.bind_text(":id", id).expect("bind");
    stmt.bind_text(":preview", preview).expect("bind");
    stmt.bind_text(":hash", &format!("hash-{id}"))
        .expect("bind");
    stmt.bind_int64(":size", preview.len() as i64)
        .expect("bind");
    stmt.bind_int64(":kind", kind.to_stored()).expect("bind");
    stmt.step().expect("insert offer");

    let mut stmt = db
        .prepare("INSERT INTO selection_fts (selection_id, content) VALUES (:id, :c)")
        .expect("prepare");
    stmt.bind_text(":id", id).expect("bind");
    stmt.bind_text(":c", content).expect("bind");
    stmt.step().expect("index content");
}

fn ids(page: &store::Page) -> Vec<&str> {
    page.data.iter().map(|e| e.id.as_str()).collect()
}

#[test]
fn an_empty_history_is_an_empty_page() {
    let s = empty();
    let page = store::query(&s.db, 10, 0, &ListSettings::default()).expect("query");
    assert!(page.data.is_empty());
    assert_eq!(page.total_count, 0);
    assert_eq!(page.total_pages, 0);
    assert_eq!(page.current_page, 0);
}

#[test]
fn rows_come_back_pinned_first_then_most_recent() {
    let s = empty();
    insert(
        &s.db,
        "old",
        "old text",
        "old text",
        OfferKind::Text,
        100,
        None,
        "",
    );
    insert(
        &s.db,
        "new",
        "new text",
        "new text",
        OfferKind::Text,
        300,
        None,
        "",
    );
    insert(
        &s.db,
        "mid",
        "mid text",
        "mid text",
        OfferKind::Text,
        200,
        None,
        "",
    );
    insert(
        &s.db,
        "pin",
        "pinned",
        "pinned",
        OfferKind::Text,
        50,
        Some(900),
        "",
    );

    let page = store::query(&s.db, 10, 0, &ListSettings::default()).expect("query");
    assert_eq!(
        ids(&page),
        vec!["pin", "new", "mid", "old"],
        "pinned_at DESC then updated_at DESC"
    );
    assert_eq!(page.total_count, 4);
}

#[test]
fn the_total_counts_every_row_not_just_the_page() {
    // COUNT(*) OVER() sits inside the subquery, where SQL evaluates it before
    // LIMIT. If it moved outside, this would report 2.
    let s = empty();
    for i in 0..5 {
        insert(
            &s.db,
            &format!("s{i}"),
            "text",
            "text",
            OfferKind::Text,
            i64::from(i),
            None,
            "",
        );
    }

    let page = store::query(&s.db, 2, 0, &ListSettings::default()).expect("query");
    assert_eq!(page.data.len(), 2, "the page is limited");
    assert_eq!(page.total_count, 5, "the count is not");
    assert_eq!(page.total_pages, 3, "ceil(5 / 2)");
}

#[test]
fn paging_walks_the_whole_history_without_repeating() {
    let s = empty();
    for i in 0..5 {
        insert(
            &s.db,
            &format!("s{i}"),
            "text",
            "text",
            OfferKind::Text,
            i64::from(i),
            None,
            "",
        );
    }

    let mut seen = Vec::new();
    for offset in (0..6).step_by(2) {
        let page = store::query(&s.db, 2, offset, &ListSettings::default()).expect("query");
        seen.extend(page.data.into_iter().map(|e| e.id));
    }
    seen.sort();
    assert_eq!(seen, vec!["s0", "s1", "s2", "s3", "s4"]);
}

#[test]
fn a_search_with_a_trigram_run_uses_fts() {
    let s = empty();
    insert(
        &s.db,
        "ff",
        "firefox",
        "firefox homepage",
        OfferKind::Text,
        10,
        None,
        "",
    );
    insert(
        &s.db,
        "gh",
        "github",
        "github dot com",
        OfferKind::Text,
        20,
        None,
        "",
    );

    let page = store::query(
        &s.db,
        10,
        0,
        &ListSettings {
            query: "fire".to_owned(),
            kind: None,
        },
    )
    .expect("query");
    assert_eq!(ids(&page), vec!["ff"]);
    assert_eq!(page.total_count, 1);
}

#[test]
fn a_two_letter_search_finds_what_fts_alone_would_miss() {
    // The whole reason search::plan exists. "gi" has no trigram run, so MATCH
    // cannot reach "github dot com"; the instr fallback can.
    let s = empty();
    insert(
        &s.db,
        "ff",
        "firefox",
        "firefox homepage",
        OfferKind::Text,
        10,
        None,
        "",
    );
    insert(
        &s.db,
        "gh",
        "github",
        "github dot com",
        OfferKind::Text,
        20,
        None,
        "",
    );

    let page = store::query(
        &s.db,
        10,
        0,
        &ListSettings {
            query: "gi".to_owned(),
            kind: None,
        },
    )
    .expect("query");
    assert_eq!(
        ids(&page),
        vec!["gh"],
        "a two-letter term must still find a longer entry, via instr"
    );
}

#[test]
fn every_word_must_match() {
    let s = empty();
    insert(
        &s.db,
        "both",
        "x",
        "alpha beta",
        OfferKind::Text,
        10,
        None,
        "",
    );
    insert(
        &s.db,
        "one",
        "x",
        "alpha gamma",
        OfferKind::Text,
        20,
        None,
        "",
    );

    let page = store::query(
        &s.db,
        10,
        0,
        &ListSettings {
            query: "alpha beta".to_owned(),
            kind: None,
        },
    )
    .expect("query");
    assert_eq!(ids(&page), vec!["both"], "terms are ANDed, not ORed");
}

#[test]
fn a_short_term_and_a_long_term_must_both_match() {
    // `every_word_must_match` above does NOT exercise the SQL-level AND:
    // "alpha" and "beta" both have trigram runs, so both go to MATCH and the
    // query gets ONE condition, with the AND inside the FTS string. Joining a
    // single condition with AND or OR is the same query, and a control proved
    // it -- flipping the join to OR left every test green.
    //
    // A short term plus a long one is the case that produces two SQL
    // conditions: one MATCH and one instr.
    let s = empty();
    insert(
        &s.db,
        "both",
        "x",
        "alpha xy",
        OfferKind::Text,
        10,
        None,
        "",
    );
    insert(
        &s.db,
        "long",
        "x",
        "alpha only",
        OfferKind::Text,
        20,
        None,
        "",
    );
    insert(
        &s.db,
        "short",
        "x",
        "xy only",
        OfferKind::Text,
        30,
        None,
        "",
    );

    let page = store::query(
        &s.db,
        10,
        0,
        &ListSettings {
            query: "alpha xy".to_owned(),
            kind: None,
        },
    )
    .expect("query");
    assert_eq!(
        ids(&page),
        vec!["both"],
        "a MATCH condition and an instr condition must be ANDed, not ORed"
    );
}

#[test]
fn a_search_and_a_kind_filter_must_both_apply() {
    // The other two-condition shape: MATCH plus `kind = ?`.
    let s = empty();
    insert(
        &s.db,
        "hit",
        "x",
        "alpha text",
        OfferKind::Text,
        10,
        None,
        "",
    );
    insert(
        &s.db,
        "wrongkind",
        "x",
        "alpha image",
        OfferKind::Image,
        20,
        None,
        "",
    );
    insert(
        &s.db,
        "wrongtext",
        "x",
        "gamma text",
        OfferKind::Text,
        30,
        None,
        "",
    );

    let page = store::query(
        &s.db,
        10,
        0,
        &ListSettings {
            query: "alpha".to_owned(),
            kind: Some(OfferKind::Text),
        },
    )
    .expect("query");
    assert_eq!(ids(&page), vec!["hit"], "both filters apply, not either");
}

#[test]
fn filtering_by_kind_needs_no_search() {
    let s = empty();
    insert(
        &s.db,
        "t",
        "text",
        "some text",
        OfferKind::Text,
        10,
        None,
        "",
    );
    insert(
        &s.db,
        "i",
        "image",
        "some image",
        OfferKind::Image,
        20,
        None,
        "",
    );

    let page = store::query(
        &s.db,
        10,
        0,
        &ListSettings {
            query: String::new(),
            kind: Some(OfferKind::Image),
        },
    )
    .expect("query");
    assert_eq!(ids(&page), vec!["i"]);
    assert_eq!(page.total_count, 1);
}

#[test]
fn a_selection_indexed_twice_appears_once() {
    // selection_fts holds a row for the content and another for the keywords,
    // so the join multiplies rows. Without GROUP BY this selection would come
    // back twice and be counted twice.
    let s = empty();
    insert(
        &s.db,
        "dup",
        "hello",
        "hello world",
        OfferKind::Text,
        10,
        None,
        "",
    );
    let mut stmt = s
        .db
        .prepare("INSERT INTO selection_fts (selection_id, content) VALUES ('dup', 'hello again')")
        .expect("prepare");
    stmt.step().expect("index a second row");

    let page = store::query(
        &s.db,
        10,
        0,
        &ListSettings {
            query: "hello".to_owned(),
            kind: None,
        },
    )
    .expect("query");
    assert_eq!(ids(&page), vec!["dup"]);
    assert_eq!(page.total_count, 1, "counted once, not twice");
}

#[test]
fn the_row_carries_every_field_the_list_view_needs() {
    let s = empty();
    insert(
        &s.db,
        "one",
        "preview text",
        "preview text",
        OfferKind::Link,
        77,
        Some(88),
        "kw",
    );

    let page = store::query(&s.db, 10, 0, &ListSettings::default()).expect("query");
    let entry = &page.data[0];
    assert_eq!(entry.id, "one");
    assert_eq!(entry.mime_type, "text/plain");
    assert_eq!(entry.text_preview, "preview text");
    assert_eq!(entry.updated_at, 77);
    assert_eq!(entry.pinned_at, 88);
    assert_eq!(entry.keywords, "kw");
    assert_eq!(entry.md5sum, "hash-one");
    assert_eq!(entry.size, "preview text".len() as i64);
    assert_eq!(entry.kind, OfferKind::Link);
    assert_eq!(entry.encryption, EncryptionType::None);
    assert_eq!(entry.url_host, None);
}

#[test]
fn a_zero_limit_is_refused_rather_than_dividing_by_it() {
    // The C++ computes totalPages as ceil(count / limit), so a zero limit is a
    // division by zero cast to int.
    let s = empty();
    assert!(matches!(
        store::query(&s.db, 0, 0, &ListSettings::default()),
        Err(Error::NonPositiveLimit(0))
    ));
    assert!(matches!(
        store::query(&s.db, -1, 0, &ListSettings::default()),
        Err(Error::NonPositiveLimit(-1))
    ));
}

#[test]
fn a_search_matching_nothing_is_an_empty_page_not_the_whole_history() {
    let s = empty();
    insert(&s.db, "a", "x", "alpha", OfferKind::Text, 10, None, "");
    insert(&s.db, "b", "x", "beta", OfferKind::Text, 20, None, "");

    let page = store::query(
        &s.db,
        10,
        0,
        &ListSettings {
            query: "zzzz".to_owned(),
            kind: None,
        },
    )
    .expect("query");
    assert!(page.data.is_empty());
    assert_eq!(page.total_count, 0);
}
