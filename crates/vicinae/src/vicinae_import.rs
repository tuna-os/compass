//! A one-shot import of Vicinae's clipboard history (ADR-0017 decision 3).
//!
//! Compass keeps its own clipboard store (`compass-clipboard.db`, under its
//! own keyring entry), so someone coming from Vicinae would otherwise start
//! with an empty history. This reads Vicinae's `clipboard.db` and payload
//! directory, decrypts with Vicinae's own key, and records each entry in
//! Compass's store with its original times, pin and keywords.
//!
//! It reads the *content* tables only (`selection`, `data_offer`), never the
//! FTS index, and never writes to Vicinae's files. It runs once: a marker
//! beside Compass's store records that it happened, and it is written only
//! after an import that reached the end, so a keyring that was locked at
//! first start gets another try at the next.

use std::path::{Path, PathBuf};

use compass_clipboard::kind::EncryptionType;
use compass_crypto::KEY_SIZE;
use compass_sqlcipher_sys::rusqlite;

use crate::clipboard_service::{ClipboardStore, Error};

/// Vicinae's clipboard database, in the shared data directory.
pub const SOURCE_DATABASE: &str = "clipboard.db";

/// Vicinae's payload directory, beside it.
pub const SOURCE_PAYLOAD_DIR: &str = "clipboard-data";

/// Written beside Compass's store once the import has run.
pub const MARKER: &str = "compass-clipboard.imported-from-vicinae";

/// One Vicinae history entry, decrypted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VicinaeEntry {
    /// The preferred offer's bytes.
    pub data: Vec<u8>,
    /// Its MIME type.
    pub mime_type: String,
    /// The application it was copied from, when Vicinae knew.
    pub source: Option<String>,
    /// When it was first copied, in Compass's milliseconds.
    pub created_at: i64,
    /// When it was last copied, in Compass's milliseconds.
    pub updated_at: i64,
    /// When it was pinned, in Compass's milliseconds; `None` when it is not.
    pub pinned_at: Option<i64>,
    /// Keywords the user gave it; empty for none.
    pub keywords: String,
}

/// What an import did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ImportReport {
    /// Entries now in Compass's history that were not before.
    pub imported: usize,
    /// Entries Compass already had (the same content), left as they were.
    pub already_present: usize,
    /// Entries that could not be read: a missing payload, one encrypted
    /// without a key to open it, or one Compass's store would not take.
    pub skipped: usize,
}

/// Vicinae's timestamps are Unix seconds; Compass's are milliseconds
/// (PARITY.md, `compass-clipboard` divergence 3). A value already past the
/// seconds range is taken as milliseconds rather than scaled twice.
#[must_use]
pub const fn to_millis(stamp: i64) -> i64 {
    const SECONDS_UNTIL_5138: i64 = 100_000_000_000;
    if stamp < SECONDS_UNTIL_5138 {
        stamp.saturating_mul(1000)
    } else {
        stamp
    }
}

/// Reads every Vicinae history entry, oldest first, calling `each` with it.
///
/// Oldest first so that, were two entries to collide in Compass's store, the
/// newer one is the one that lands last. Entries are handed over one at a
/// time rather than collected, because a history can hold many images.
/// Returns how many entries could not be read. `master` is Vicinae's master
/// key (from its own keyring entry); `None` reads an unencrypted database and
/// skips encrypted payloads.
///
/// # Errors
///
/// [`Error::Database`] when the database exists but will not open or
/// answer — most often, a keyed database and no key.
pub fn read_clipboard(
    vicinae_dir: &Path,
    master: Option<&[u8; KEY_SIZE]>,
    each: &mut dyn FnMut(VicinaeEntry),
) -> Result<usize, Error> {
    let path = vicinae_dir.join(SOURCE_DATABASE);
    if !path.is_file() {
        return Ok(0);
    }
    let keys = master.map(|master| compass_crypto::keys::derive_all(master));
    let database_key: &[u8] = keys.as_ref().map_or(&[], |keys| &keys.database);
    let db =
        compass_sqlcipher_sys::open(&path, database_key).map_err(|err| database(&path, &err))?;
    let payload_dir = vicinae_dir.join(SOURCE_PAYLOAD_DIR);

    let mut stmt = db
        .prepare(
            "SELECT id, source, created_at, updated_at, pinned_at, keywords \
             FROM selection ORDER BY updated_at ASC, rowid ASC",
        )
        .map_err(|err| database(&path, &err))?;
    let mut rows = stmt.query([]).map_err(|err| database(&path, &err))?;
    let mut skipped = 0;
    while let Some(row) = rows.next().map_err(|err| database(&path, &err))? {
        let columns = selection_columns(row).map_err(|err| database(&path, &err))?;
        let Some(id) = columns.id else {
            skipped += 1;
            continue;
        };
        let offer = match compass_clipboard::write::find_preferred_offer(&db, &id) {
            Ok(Some(offer)) => offer,
            Ok(None) | Err(_) => {
                skipped += 1;
                continue;
            }
        };
        let Some(data) = payload(
            &payload_dir,
            &offer.id,
            offer.encryption,
            keys.as_ref().map(|keys| &keys.clipboard),
        ) else {
            skipped += 1;
            continue;
        };
        each(VicinaeEntry {
            data,
            mime_type: offer.mime_type,
            source: columns.source.filter(|source| !source.is_empty()),
            created_at: to_millis(columns.created_at),
            updated_at: to_millis(columns.updated_at),
            pinned_at: columns.pinned_at.map(to_millis),
            keywords: columns.keywords,
        });
    }
    Ok(skipped)
}

/// One `selection` row as the import reads it, NULLs read the way
/// `QVariant` reads them: absent text, zero times.
struct SelectionColumns {
    id: Option<String>,
    source: Option<String>,
    created_at: i64,
    updated_at: i64,
    pinned_at: Option<i64>,
    keywords: String,
}

fn selection_columns(row: &rusqlite::Row<'_>) -> rusqlite::Result<SelectionColumns> {
    Ok(SelectionColumns {
        id: row.get(0)?,
        source: row.get(1)?,
        created_at: row.get::<_, Option<i64>>(2)?.unwrap_or(0),
        updated_at: row.get::<_, Option<i64>>(3)?.unwrap_or(0),
        pinned_at: row.get(4)?,
        keywords: row.get::<_, Option<String>>(5)?.unwrap_or_default(),
    })
}

fn payload(
    dir: &Path,
    offer_id: &str,
    encryption: EncryptionType,
    key: Option<&[u8; KEY_SIZE]>,
) -> Option<Vec<u8>> {
    let stored = std::fs::read(compass_clipboard::ingest::payload_path(dir, offer_id)).ok()?;
    match encryption {
        EncryptionType::None => Some(stored),
        EncryptionType::Local => compass_crypto::decrypt(&stored, key?).ok(),
    }
}

fn database(path: &Path, err: &rusqlite::Error) -> Error {
    Error::Database(format!("{}: {err}", path.display()))
}

/// Imports Vicinae's history into `store` unless that already happened.
///
/// `compass_dir` is where Compass's store and the marker live; `vicinae_dir`
/// is where Vicinae keeps `clipboard.db`. They are the same directory in a
/// real install and separate in tests. Returns `None` when the marker says
/// the import already ran.
///
/// # Errors
///
/// [`Error::Database`] when Vicinae's database will not open, and
/// [`Error::Store`] when the marker cannot be written. Neither writes the
/// marker, so the next start tries again.
pub fn import_once(
    store: &ClipboardStore,
    compass_dir: &Path,
    vicinae_dir: &Path,
    master: Option<&[u8; KEY_SIZE]>,
) -> Result<Option<ImportReport>, Error> {
    let marker = marker_path(compass_dir);
    if marker.exists() {
        return Ok(None);
    }
    let mut report = ImportReport::default();
    report.skipped = read_clipboard(
        vicinae_dir,
        master,
        &mut |entry| match store.import(&entry) {
            Ok(true) => report.imported += 1,
            Ok(false) => report.already_present += 1,
            Err(err) => {
                tracing::warn!(error = %err, "a Vicinae clipboard entry was not imported");
                report.skipped += 1;
            }
        },
    )?;
    std::fs::write(
        &marker,
        format!(
            "imported={} already_present={} skipped={}\n",
            report.imported, report.already_present, report.skipped
        ),
    )
    .map_err(|err| Error::Store(format!("writing {}: {err}", marker.display())))?;
    Ok(Some(report))
}

/// Where the marker for `compass_dir` goes.
#[must_use]
pub fn marker_path(compass_dir: &Path) -> PathBuf {
    compass_dir.join(MARKER)
}

#[cfg(test)]
mod tests {
    use super::*;

    const VICINAE_MASTER: [u8; KEY_SIZE] = [7; KEY_SIZE];
    const COMPASS_MASTER: [u8; KEY_SIZE] = [9; KEY_SIZE];

    /// A Vicinae data directory: its schema, keyed and encrypted the way the
    /// C++ does it, with times in seconds. `(text, updated_s, pinned_s,
    /// keywords)`.
    fn vicinae_history(dir: &Path, entries: &[(&str, i64, Option<i64>, &str)]) {
        let keys = compass_crypto::keys::derive_all(&VICINAE_MASTER);
        let db =
            compass_sqlcipher_sys::open(&dir.join(SOURCE_DATABASE), &keys.database).expect("open");
        compass_clipboard::schema::run(&db).expect("schema");
        let mut n = 0;
        for (text, updated, pinned, keywords) in entries {
            let decision = compass_clipboard::ingest::ingest(
                &db,
                &dir.join(SOURCE_PAYLOAD_DIR),
                &compass_clipboard::ingest::Incoming {
                    data: text.as_bytes(),
                    mime_type: "text/plain",
                    source_app: Some("org.gnome.TextEditor"),
                },
                Some(&keys.clipboard),
                &mut || {
                    n += 1;
                    format!("vicinae-{n}")
                },
            )
            .expect("ingest");
            let compass_clipboard::ingest::Decision::Inserted { selection_id, .. } = decision
            else {
                panic!("fixture entry {text:?} was not inserted");
            };
            db.execute(
                "UPDATE selection SET created_at = :t, updated_at = :t, pinned_at = :p \
                 WHERE id = :id",
                rusqlite::named_params! { ":t": updated, ":p": pinned, ":id": selection_id },
            )
            .expect("update");
            if !keywords.is_empty() {
                compass_clipboard::write::set_keywords(&db, &selection_id, keywords)
                    .expect("keywords");
            }
        }
    }

    fn previews(store: &ClipboardStore, query: &str) -> Vec<(String, bool)> {
        store
            .history(query, 50)
            .expect("history")
            .into_iter()
            .map(|entry| (entry.preview, entry.pinned))
            .collect()
    }

    #[test]
    fn history_comes_across_decrypted_with_its_order_pins_and_keywords() {
        let vicinae = tempfile::tempdir().expect("tempdir");
        let compass = tempfile::tempdir().expect("tempdir");
        vicinae_history(
            vicinae.path(),
            &[
                ("oldest", 1_700_000_000, None, ""),
                ("pinned long ago", 1_700_000_100, Some(1_700_000_200), ""),
                ("newest", 1_700_000_300, None, "ticket"),
            ],
        );
        let store = ClipboardStore::open(compass.path(), &COMPASS_MASTER).expect("open");

        let report = import_once(
            &store,
            compass.path(),
            vicinae.path(),
            Some(&VICINAE_MASTER),
        )
        .expect("import")
        .expect("ran");
        assert_eq!(
            report,
            ImportReport {
                imported: 3,
                already_present: 0,
                skipped: 0
            }
        );
        assert_eq!(
            previews(&store, ""),
            [
                ("pinned long ago".to_owned(), true),
                ("newest".to_owned(), false),
                ("oldest".to_owned(), false),
            ],
            "pinned first, then by Vicinae's times, not by the moment of import"
        );
        assert_eq!(previews(&store, "ticket"), [("newest".to_owned(), false)]);
        let newest = store.history("newest", 1).expect("history").remove(0);
        assert_eq!(
            newest.updated_at, 1_700_000_300_000,
            "seconds became milliseconds"
        );
        let (_, data) = store.content(&newest.id).expect("read").expect("present");
        assert_eq!(
            data, b"newest",
            "re-encrypted under Compass's key, and readable"
        );
    }

    #[test]
    fn it_runs_once_and_leaves_what_compass_already_had() {
        let vicinae = tempfile::tempdir().expect("tempdir");
        let compass = tempfile::tempdir().expect("tempdir");
        vicinae_history(vicinae.path(), &[("shared", 1_700_000_000, None, "")]);
        let store = ClipboardStore::open(compass.path(), &COMPASS_MASTER).expect("open");
        store
            .record(b"shared", "text/plain", None)
            .expect("recorded");
        let before = store.history("", 5).expect("history").remove(0).updated_at;

        let report = import_once(
            &store,
            compass.path(),
            vicinae.path(),
            Some(&VICINAE_MASTER),
        )
        .expect("import")
        .expect("ran");
        assert_eq!((report.imported, report.already_present), (0, 1));
        let after = store.history("", 5).expect("history");
        assert_eq!(after.len(), 1);
        assert_eq!(
            after[0].updated_at, before,
            "not bubbled to the import time"
        );

        assert!(marker_path(compass.path()).exists());
        assert_eq!(
            import_once(
                &store,
                compass.path(),
                vicinae.path(),
                Some(&VICINAE_MASTER)
            )
            .expect("second"),
            None,
            "the marker stops a second import"
        );
    }

    #[test]
    fn a_locked_database_is_an_error_and_is_tried_again_next_time() {
        let vicinae = tempfile::tempdir().expect("tempdir");
        let compass = tempfile::tempdir().expect("tempdir");
        vicinae_history(vicinae.path(), &[("secret", 1_700_000_000, None, "")]);
        let store = ClipboardStore::open(compass.path(), &COMPASS_MASTER).expect("open");

        let wrong = [1; KEY_SIZE];
        assert!(import_once(&store, compass.path(), vicinae.path(), Some(&wrong)).is_err());
        assert!(import_once(&store, compass.path(), vicinae.path(), None).is_err());
        assert!(
            !marker_path(compass.path()).exists(),
            "no marker, so it retries"
        );
        assert!(previews(&store, "").is_empty());
    }

    #[test]
    fn no_vicinae_history_is_an_empty_import_not_an_error() {
        let vicinae = tempfile::tempdir().expect("tempdir");
        let compass = tempfile::tempdir().expect("tempdir");
        let store = ClipboardStore::open(compass.path(), &COMPASS_MASTER).expect("open");
        let report = import_once(&store, compass.path(), vicinae.path(), None)
            .expect("import")
            .expect("ran");
        assert_eq!(report, ImportReport::default());
        assert!(
            !vicinae.path().join(SOURCE_DATABASE).exists(),
            "reading must not create Vicinae's database"
        );
    }

    #[test]
    fn an_entry_whose_payload_is_gone_is_skipped_not_fatal() {
        let vicinae = tempfile::tempdir().expect("tempdir");
        let compass = tempfile::tempdir().expect("tempdir");
        vicinae_history(
            vicinae.path(),
            &[
                ("kept", 1_700_000_000, None, ""),
                ("lost", 1_700_000_100, None, ""),
            ],
        );
        std::fs::remove_file(vicinae.path().join(SOURCE_PAYLOAD_DIR).join("vicinae-4"))
            .expect("the second entry's offer");
        let store = ClipboardStore::open(compass.path(), &COMPASS_MASTER).expect("open");
        let report = import_once(
            &store,
            compass.path(),
            vicinae.path(),
            Some(&VICINAE_MASTER),
        )
        .expect("import")
        .expect("ran");
        assert_eq!((report.imported, report.skipped), (1, 1));
        assert_eq!(previews(&store, ""), [("kept".to_owned(), false)]);
    }

    #[test]
    fn seconds_become_milliseconds_and_milliseconds_stay() {
        assert_eq!(to_millis(1_700_000_000), 1_700_000_000_000);
        assert_eq!(to_millis(1_700_000_000_000), 1_700_000_000_000);
        assert_eq!(to_millis(0), 0);
    }
}
