//! One observed copy becomes history — against real encrypted databases.
//!
//! These drive [`compass_clipboard::ingest`] the way the daemon loop will:
//! the bodies the GNOME helper extension emits on `ClipboardChanged`
//! (single offer: bytes, MIME type, best-effort source app) go in, history
//! rows and payload files come out.

use compass_clipboard::ingest::{self, Decision, IgnoreReason, Incoming};
use compass_clipboard::kind::{EncryptionType, OfferKind};
use compass_clipboard::{schema, store};
use compass_sqlcipher_sys::Database;

const KEY: &[u8] = &[0x44; 32];
const CLIPBOARD_KEY: [u8; compass_crypto::KEY_SIZE] = [0x22; compass_crypto::KEY_SIZE];

struct Home {
    _dir: tempfile::TempDir,
    db: Database,
    data_dir: std::path::PathBuf,
}

fn fresh() -> Home {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let db = Database::open(&dir.path().join("clip.db"), KEY).expect("open");
    schema::run(&db).expect("migrations");
    // Deliberately *not* created: ingest owns `create_dir_all`.
    let data_dir = dir.path().join("payloads");
    Home {
        _dir: dir,
        db,
        data_dir,
    }
}

/// Deterministic ids: `sel-0`, `off-1`, … Selection first, then offer.
fn ids() -> impl FnMut() -> String {
    let mut next = 0;
    move || {
        let id = if next % 2 == 0 {
            format!("sel-{}", next / 2)
        } else {
            format!("off-{}", next / 2)
        };
        next += 1;
        id
    }
}

fn text(data: &str) -> Incoming<'_> {
    Incoming {
        data: data.as_bytes(),
        mime_type: "text/plain;charset=utf-8",
        source_app: Some("org.gnome.Terminal"),
    }
}

#[test]
fn a_text_copy_is_stored_searchable_and_on_disk_verbatim() {
    let home = fresh();
    let decision = ingest::ingest(
        &home.db,
        &home.data_dir,
        &text("hello world"),
        None,
        &mut ids(),
    )
    .expect("ingest");
    assert_eq!(
        decision,
        Decision::Inserted {
            selection_id: "sel-0".to_owned(),
            offer_id: "off-0".to_owned(),
        }
    );

    let page = store::query(&home.db, 10, 0, &store::ListSettings::default()).expect("query");
    assert_eq!(page.total_count, 1);
    let row = &page.data[0];
    assert_eq!(row.id, "sel-0");
    assert_eq!(row.text_preview, "hello world");
    assert_eq!(row.kind, OfferKind::Text);
    assert_eq!(row.encryption, EncryptionType::None);

    // Searchable, which is index_content's doing.
    let hits = store::query(
        &home.db,
        10,
        0,
        &store::ListSettings {
            query: "wor".to_owned(),
            kind: None,
        },
    )
    .expect("query");
    assert_eq!(hits.data.len(), 1);

    // Unencrypted payloads are stored verbatim under the offer id.
    let raw = std::fs::read(home.data_dir.join("off-0")).expect("payload file");
    assert_eq!(raw, b"hello world");
}

#[test]
fn copying_the_same_thing_twice_bubbles_instead_of_inserting() {
    let home = fresh();
    let first =
        ingest::ingest(&home.db, &home.data_dir, &text("again"), None, &mut ids()).expect("ingest");
    assert!(matches!(first, Decision::Inserted { .. }));

    // Bubble-up must not mint ids: a counter that panics on use proves it.
    let mut explode = || -> String { panic!("bubble-up must not mint ids") };
    let second = ingest::ingest(&home.db, &home.data_dir, &text("again"), None, &mut explode)
        .expect("ingest");
    assert_eq!(second, Decision::BubbledUp);

    let page = store::query(&home.db, 10, 0, &store::ListSettings::default()).expect("query");
    assert_eq!(page.total_count, 1, "no duplicate row");
}

#[test]
fn nothing_offerable_blank_text_and_unknown_kinds_are_ignored() {
    let home = fresh();
    let cases: &[(&[u8], &str, IgnoreReason)] = &[
        (b"", "text/plain", IgnoreReason::Empty),
        (b"   \n  ", "text/plain", IgnoreReason::BlankText),
        (
            b"\x00\x01\x02",
            "application/octet-stream",
            IgnoreReason::UnknownKind,
        ),
    ];
    for (data, mime, reason) in cases {
        let decision = ingest::ingest(
            &home.db,
            &home.data_dir,
            &Incoming {
                data,
                mime_type: mime,
                source_app: None,
            },
            None,
            &mut ids(),
        )
        .expect("ingest never fails an ignorable copy");
        assert_eq!(decision, Decision::Ignored(*reason), "for {mime:?}");
    }

    let page = store::query(&home.db, 10, 0, &store::ListSettings::default()).expect("query");
    assert_eq!(page.total_count, 0);
}

#[test]
fn an_encrypted_payload_decrypts_with_the_clipboard_key() {
    let home = fresh();
    let decision = ingest::ingest(
        &home.db,
        &home.data_dir,
        &text("secret"),
        Some(&CLIPBOARD_KEY),
        &mut ids(),
    )
    .expect("ingest");
    let Decision::Inserted { offer_id, .. } = decision else {
        panic!("expected an insert, got {decision:?}");
    };

    let page = store::query(&home.db, 10, 0, &store::ListSettings::default()).expect("query");
    assert_eq!(page.data[0].encryption, EncryptionType::Local);

    // `[iv | ciphertext | tag]`, per the format compass-crypto documents —
    // which is what makes this row readable by the C++ engine too.
    let sealed = std::fs::read(home.data_dir.join(offer_id)).expect("payload file");
    assert_ne!(sealed, b"secret");
    let open = compass_crypto::decrypt(&sealed, &CLIPBOARD_KEY).expect("decrypt");
    assert_eq!(open, b"secret");
}

#[test]
fn links_files_and_images_classify_like_the_cpp_service() {
    let home = fresh();
    let mut id_gen = ids();
    for (data, mime) in [
        ("https://example.com/x", "text/plain"),
        ("file:///home/a", "text/plain"),
        ("file:///home/a\r\nfile:///home/b\r\n", "text/uri-list"),
        ("<b>hi</b>", "text/html"),
    ] {
        let decision = ingest::ingest(
            &home.db,
            &home.data_dir,
            &Incoming {
                data: data.as_bytes(),
                mime_type: mime,
                source_app: None,
            },
            None,
            &mut id_gen,
        )
        .expect("ingest");
        assert!(
            matches!(decision, Decision::Inserted { .. }),
            "for {mime:?}"
        );
    }

    let page = store::query(&home.db, 100, 0, &store::ListSettings::default()).expect("query");
    let mut kinds: Vec<OfferKind> = page.data.iter().map(|row| row.kind).collect();
    kinds.sort_by_key(|k| *k as u8);
    assert_eq!(
        kinds,
        vec![
            OfferKind::Text,
            OfferKind::Link,
            OfferKind::File,
            OfferKind::File
        ]
    );

    let link = page
        .data
        .iter()
        .find(|row| row.kind == OfferKind::Link)
        .expect("a link row");
    assert_eq!(link.url_host.as_deref(), Some("example.com"));

    // Images are stored but never indexed as text.
    let decision = ingest::ingest(
        &home.db,
        &home.data_dir,
        &Incoming {
            data: &[0x89, 0x50, 0x4e, 0x47],
            mime_type: "image/png",
            source_app: None,
        },
        None,
        &mut id_gen,
    )
    .expect("ingest");
    let Decision::Inserted { selection_id, .. } = decision else {
        panic!("expected an image insert");
    };
    let found = compass_clipboard::write::find_preferred_offer(&home.db, &selection_id)
        .expect("lookup")
        .expect("the image offer");
    assert_eq!(found.mime_type, "image/png");
}
