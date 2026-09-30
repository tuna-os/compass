//! Writing clipboard history — inserts, indexing, deletion, and bubbling up.
//!
//! Ports the write half of `ClipboardDatabase`. Two of these fix bugs rather
//! than reproduce them; both are recorded in
//! [PARITY.md](../../../docs/rust-engine/PARITY.md) and both have a test.

use compass_sqlcipher_sys::rusqlite::{
    self, Connection, OptionalExtension as _, Params, Transaction, named_params,
};

use crate::kind::{EncryptionType, OfferKind};

/// How much of a selection's text is indexed for search.
///
/// `MAX_INDEXED_CONTENT_SIZE` in `clipboard-db.cpp`, where it is applied as
/// `content.left(...)`. `QString::left` counts **UTF-16 code units**, not
/// characters and not bytes, so [`index_content`] truncates the same way — see
/// its docs for the one place this cannot follow exactly.
pub const MAX_INDEXED_CONTENT: usize = 1 << 16;

/// The selection insert.
const INSERT_SELECTION: &str = "INSERT INTO selection (id, kind, offer_count, hash_md5, \
     preferred_mime_type, source, created_at, updated_at) \
     VALUES (:id, :kind, :offer_count, :hash_md5, :preferred_mime_type, :source, :epoch, :epoch)";

/// The offer insert.
const INSERT_OFFER: &str = "INSERT INTO data_offer (id, selection_id, mime_type, text_preview, \
     content_hash_md5, encryption_type, size, kind, url_host) \
     VALUES (:id, :selection_id, :mime_type, :text_preview, :content_hash_md5, :encryption, \
     :size, :kind, :url_host)";

/// The search-index insert.
const INSERT_INDEXED_CONTENT: &str =
    "INSERT INTO selection_fts (selection_id, content) VALUES (:id, :content)";

/// What to record about a new selection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewSelection<'a> {
    /// A fresh UUID.
    pub id: &'a str,
    /// How many offers accompany it.
    pub offer_count: i64,
    /// The MD5 the service computed over the selection.
    pub hash: &'a str,
    /// Which offer's MIME type the list view should show.
    pub preferred_mime_type: &'a str,
    /// What kind of thing it is.
    pub kind: OfferKind,
    /// The application it came from, if known.
    pub source: Option<&'a str>,
}

/// What to record about one offer of a selection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewOffer<'a> {
    /// A fresh UUID.
    pub id: &'a str,
    /// The selection this belongs to.
    pub selection_id: &'a str,
    /// The MIME type this offer is in.
    pub mime_type: &'a str,
    /// A short preview for the list view.
    pub text_preview: &'a str,
    /// The MD5 of the payload.
    pub md5sum: &'a str,
    /// Whether the payload on disk is encrypted.
    pub encryption: EncryptionType,
    /// What kind of thing it is.
    pub kind: OfferKind,
    /// The payload's size in bytes.
    pub size: i64,
    /// For links, the host, so a favicon can be fetched without parsing.
    pub url_host: Option<&'a str>,
}

/// Errors from writing history.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// SQLite refused.
    #[error(transparent)]
    Database(#[from] rusqlite::Error),
}

type Result<T> = std::result::Result<T, Error>;

/// Now, in milliseconds since the epoch.
///
/// Milliseconds, not the C++ engine's whole seconds. Compass owns this store
/// (ADR-0017), and at second granularity two copies made within the same
/// second tie on `updated_at`, so history could show the older one on top.
/// A Vicinae importer multiplies its seconds by 1000.
///
/// Taken once per call and bound, rather than left to SQLite's `unixepoch()`.
/// That is not a style preference: see [`evict_older_than`].
pub(crate) fn now() -> i64 {
    i64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
    )
    .unwrap_or(i64::MAX)
}

/// The `updated_at` for a copy happening now: never equal to or below one
/// already stored.
///
/// A clock alone cannot order history. Two copies in the same millisecond tie,
/// and re-copying an old entry the moment after something new must still put
/// it on top; a tie-break by insertion order gets that second case wrong,
/// because a bubbled-up entry keeps its old row. So each copy is stamped
/// `max(now, newest + 1)`: strictly after everything before it, and equal to
/// the wall clock whenever the clock is ahead (which is all but always).
fn next_stamp(db: &Connection) -> Result<i64> {
    let newest: Option<i64> = db.query_row("SELECT MAX(updated_at) FROM selection", [], |row| {
        row.get(0)
    })?;
    Ok(newest.map_or_else(now, |newest| now().max(newest.saturating_add(1))))
}

/// Record a new selection. `created_at` and `updated_at` are both set to now,
/// or just after the newest entry if the clock has not moved past it.
///
/// # Errors
///
/// Returns [`Error::Database`] if the insert fails — including on a duplicate
/// id, which the primary key rejects.
pub fn insert_selection(db: &Connection, selection: &NewSelection<'_>) -> Result<()> {
    let epoch = next_stamp(db)?;
    db.prepare_cached(INSERT_SELECTION)?
        .execute(named_params! {
            ":id": selection.id,
            ":kind": selection.kind.to_stored(),
            ":offer_count": selection.offer_count,
            ":hash_md5": selection.hash,
            ":preferred_mime_type": selection.preferred_mime_type,
            ":source": selection.source,
            ":epoch": epoch,
        })?;
    Ok(())
}

/// Record one offer of a selection.
///
/// # Errors
///
/// Returns [`Error::Database`] if the insert fails.
pub fn insert_offer(db: &Connection, offer: &NewOffer<'_>) -> Result<()> {
    db.prepare_cached(INSERT_OFFER)?.execute(named_params! {
        ":id": offer.id,
        ":selection_id": offer.selection_id,
        ":mime_type": offer.mime_type,
        ":text_preview": offer.text_preview,
        ":content_hash_md5": offer.md5sum,
        ":encryption": offer.encryption.to_stored(),
        ":size": offer.size,
        ":kind": offer.kind.to_stored(),
        ":url_host": offer.url_host,
    })?;
    Ok(())
}

/// Truncate `content` the way `QString::left(MAX_INDEXED_CONTENT_SIZE)` does.
///
/// `QString::left` counts UTF-16 code units. A cut at exactly the limit can
/// therefore land between the two halves of a surrogate pair, and Qt's
/// subsequent `toUtf8()` turns the orphaned half into a replacement character.
/// This drops the split character instead of emitting a replacement for it:
/// the difference is one character at the 65536-unit boundary of an entry
/// longer than that, and reproducing Qt's mangling would mean writing invalid
/// text into the index on purpose.
#[must_use]
pub fn truncate_for_index(content: &str) -> &str {
    let mut units = 0usize;
    for (offset, ch) in content.char_indices() {
        let width = ch.len_utf16();
        if units + width > MAX_INDEXED_CONTENT {
            return &content[..offset];
        }
        units += width;
    }
    content
}

/// Index a selection's text so it can be searched.
///
/// # Errors
///
/// Returns [`Error::Database`] if the insert fails.
pub fn index_content(db: &Connection, selection_id: &str, content: &str) -> Result<()> {
    db.prepare_cached(INSERT_INDEXED_CONTENT)?
        .execute(named_params! {
            ":id": selection_id,
            ":content": truncate_for_index(content),
        })?;
    Ok(())
}

/// Move an existing selection to the top of the history, by id or by content
/// hash.
///
/// Returns whether anything was moved — `false` means no such selection, and
/// the caller should insert one.
///
/// # This does not use `sqlite3_changes`
///
/// `tryBubbleUpSelection` runs the `UPDATE`, **discards whether it succeeded**
/// (it only logs), and answers from `m_db.changes()` — a connection-wide
/// counter reporting the most recent *successful* statement. If the update
/// never ran, that counter still holds whatever the previous write changed, so
/// the function can return `true` having moved nothing. Its caller
/// (`clipboard-service.cpp:508`) then skips `insertSelection` entirely, and the
/// thing the user copied never reaches the history.
///
/// `RETURNING id` answers the actual question — did *this* statement touch a
/// row — per statement rather than per connection. Nothing here calls
/// `Connection::changes`, which reads that same connection-wide counter, so the
/// original shape is not reproduced by accident.
///
/// # Errors
///
/// Returns [`Error::Database`] if the update fails, rather than reporting a
/// stale success.
pub fn bubble_up(db: &Connection, id_or_hash: &str) -> Result<bool> {
    let updated_at = next_stamp(db)?;
    returns_a_row(
        db,
        "UPDATE selection SET updated_at = :updated_at \
         WHERE hash_md5 = :id OR id = :id RETURNING id",
        named_params! { ":id": id_or_hash, ":updated_at": updated_at },
    )
}

/// Whether a `RETURNING` statement produced any row. SQLite applies every
/// change on the first step, so one step is all it takes.
fn returns_a_row(db: &Connection, sql: &str, params: impl Params) -> Result<bool> {
    let mut stmt = db.prepare_cached(sql)?;
    let found = stmt.query(params)?.next()?.is_some();
    Ok(found)
}

/// The text of the first column of every row `sql` returns, NULLs skipped.
fn ids(db: &Connection, sql: &str, params: impl Params) -> Result<Vec<String>> {
    let mut stmt = db.prepare(sql)?;
    let rows = stmt.query_map(params, |row| row.get::<_, Option<String>>(0))?;
    let mut out = Vec::new();
    for id in rows {
        out.extend(id?);
    }
    Ok(out)
}

/// Delete a selection and its offers, returning the offer ids whose payloads
/// the caller must now unlink from disk.
///
/// # Errors
///
/// Returns [`Error::Database`] if either delete fails, in which case nothing is
/// committed.
pub fn remove_selection(db: &Connection, selection_id: &str) -> Result<Vec<String>> {
    let tx = db.unchecked_transaction()?;
    let removed = ids(
        &tx,
        "DELETE FROM data_offer WHERE selection_id = :id RETURNING id",
        named_params! { ":id": selection_id },
    )?;
    tx.execute(
        "DELETE FROM selection WHERE id = :id",
        named_params! { ":id": selection_id },
    )?;
    tx.commit()?;
    Ok(removed)
}

/// Delete everything older than `age`, returning the offer ids whose payloads
/// the caller must unlink from disk.
///
/// With `preserve_tagged`, pinned selections and those with keywords are kept.
///
/// # One cutoff, bound to both statements
///
/// `evictOlderThan` runs a `SELECT` to collect the offer ids and then a
/// `DELETE` to remove the selections, and computes the cutoff with
/// `unixepoch()` in **each**. SQLite holds a time function constant within a
/// statement but not across statements, and an open transaction does not freeze
/// it — measured: inside `BEGIN`, `unixepoch('subsec')` advanced after 434
/// consecutive statements. Time only moves forward, so the `DELETE` set is a
/// superset of the `SELECT` set, and anything crossing the threshold in between
/// has its rows removed while its offer ids are never returned. Those payloads
/// are then unreferenced *and* unreported: they stay on disk forever.
///
/// The window is short. Eviction runs on a timer for the life of the install
/// and the payloads include clipboard images, so the leak is unbounded rather
/// than small. Here the cutoff is computed once and bound to both statements.
///
/// # Errors
///
/// Returns [`Error::Database`] if either statement fails, in which case nothing
/// is committed.
pub fn evict_older_than(
    db: &Connection,
    age: std::time::Duration,
    preserve_tagged: bool,
) -> Result<Vec<String>> {
    const PRESERVE: &str = " AND pinned_at IS NULL AND keywords == ''";

    // One reading of the clock, used by both statements below.
    let cutoff = now().saturating_sub(i64::try_from(age.as_millis()).unwrap_or(i64::MAX));

    let tx = db.unchecked_transaction()?;

    let mut select = String::from(
        "SELECT o.id FROM data_offer o \
         JOIN selection s ON s.id = o.selection_id \
         WHERE s.updated_at < :cutoff",
    );
    if preserve_tagged {
        select.push_str(PRESERVE);
    }
    let evicted = ids(&tx, &select, named_params! { ":cutoff": cutoff })?;

    if evicted.is_empty() {
        return Ok(evicted);
    }

    let mut delete = String::from("DELETE FROM selection WHERE updated_at < :cutoff");
    if preserve_tagged {
        delete.push_str(PRESERVE);
    }
    tx.execute(&delete, named_params! { ":cutoff": cutoff })?;

    tx.commit()?;
    Ok(evicted)
}

/// Delete every selection, returning the offer ids to unlink.
///
/// # Errors
///
/// Returns [`Error::Database`] if either statement fails.
pub fn remove_all(db: &Connection, preserve_tagged: bool) -> Result<Vec<String>> {
    let tx = db.unchecked_transaction()?;

    let (select, delete) = if preserve_tagged {
        (
            "SELECT o.id FROM data_offer o JOIN selection s ON s.id = o.selection_id \
             WHERE s.pinned_at IS NULL AND s.keywords == ''",
            "DELETE FROM selection WHERE pinned_at IS NULL AND keywords == ''",
        )
    } else {
        ("SELECT id FROM data_offer", "DELETE FROM selection")
    };

    let removed = ids(&tx, select, [])?;
    tx.execute(delete, [])?;

    tx.commit()?;
    Ok(removed)
}

/// Borrow of a transaction so callers can group several writes.
///
/// Re-exported so a caller does not have to name `rusqlite` to batch an insert
/// of a selection and its offers.
pub type Batch<'db> = Transaction<'db>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncation_counts_utf16_code_units() {
        // Under the limit: unchanged.
        assert_eq!(truncate_for_index("hello"), "hello");

        // A string of exactly MAX_INDEXED_CONTENT units survives whole.
        let exact = "a".repeat(MAX_INDEXED_CONTENT);
        assert_eq!(truncate_for_index(&exact).len(), MAX_INDEXED_CONTENT);

        // One past it loses exactly one unit.
        let over = "a".repeat(MAX_INDEXED_CONTENT + 1);
        assert_eq!(truncate_for_index(&over).len(), MAX_INDEXED_CONTENT);
    }

    #[test]
    fn a_surrogate_pair_is_not_split() {
        // MAX - 1 ASCII characters leaves room for one unit, and an emoji needs
        // two. Qt would cut between the halves; this drops the character.
        let mut s = "a".repeat(MAX_INDEXED_CONTENT - 1);
        s.push('😀');
        let out = truncate_for_index(&s);
        assert_eq!(out.chars().count(), MAX_INDEXED_CONTENT - 1);
        assert!(!out.ends_with('😀'), "the emoji does not fit in one unit");

        // With room for both halves it survives.
        let mut s = "a".repeat(MAX_INDEXED_CONTENT - 2);
        s.push('😀');
        assert!(truncate_for_index(&s).ends_with('😀'));
    }

    #[test]
    fn counting_chars_instead_of_units_would_differ() {
        // The control for the two tests above: a char-counting truncation
        // admits MAX characters, which for non-BMP text is 2 * MAX units --
        // twice what the C++ indexes.
        let s = "😀".repeat(MAX_INDEXED_CONTENT);
        let ours = truncate_for_index(&s);
        assert_eq!(
            ours.chars().count(),
            MAX_INDEXED_CONTENT / 2,
            "each emoji costs two UTF-16 units"
        );
        assert_ne!(
            ours.chars().count(),
            MAX_INDEXED_CONTENT,
            "if this were equal, the truncation would be counting characters"
        );
    }
}

/// Pin a selection, or unpin it.
///
/// Returns whether a selection with that id existed.
///
/// # Why this reports "found" where the C++ reports "the statement ran"
///
/// `setPinned` returns `exec()`, which is true for an `UPDATE` that matched no
/// rows — pinning an id that does not exist "succeeds". That is a different
/// question from the one a caller asks. `RETURNING id` answers the one they
/// mean, and unlike [`bubble_up`] nothing depends on the looser reading, so
/// this is a narrowing rather than a fix.
///
/// # Errors
///
/// Returns [`Error::Database`] if the update fails.
pub fn set_pinned(db: &Connection, id: &str, pinned: bool) -> Result<bool> {
    if pinned {
        returns_a_row(
            db,
            "UPDATE selection SET pinned_at = :epoch WHERE id = :id RETURNING id",
            named_params! { ":epoch": now(), ":id": id },
        )
    } else {
        returns_a_row(
            db,
            "UPDATE selection SET pinned_at = NULL WHERE id = :id RETURNING id",
            named_params! { ":id": id },
        )
    }
}

/// Set a selection's keywords.
///
/// Returns whether a selection with that id existed.
///
/// # The trigger this fires
///
/// `selection_auk` (`002_trigram_fts.sql`) runs `AFTER UPDATE OF keywords`: it
/// deletes the `selection_fts` row whose content equals the *old* keywords and
/// inserts one holding the new. So changing keywords silently re-indexes the
/// entry, and searching by an old keyword stops finding it. That is the
/// database's behaviour rather than this function's, which is exactly why it is
/// worth a test — nothing in this file would reveal it.
///
/// # Errors
///
/// Returns [`Error::Database`] if the update fails.
pub fn set_keywords(db: &Connection, id: &str, keywords: &str) -> Result<bool> {
    returns_a_row(
        db,
        "UPDATE selection SET keywords = :kw WHERE id = :id RETURNING id",
        named_params! { ":kw": keywords, ":id": id },
    )
}

/// A selection's keywords, or `None` if there is no such selection.
///
/// A selection that exists with no keywords reads back as `Some("")`, because
/// the column defaults to the empty string rather than to NULL.
///
/// # Errors
///
/// Returns [`Error::Database`] if the query fails.
pub fn keywords_of(db: &Connection, id: &str) -> Result<Option<String>> {
    let keywords = db
        .query_row(
            "SELECT keywords FROM selection WHERE id = :id",
            named_params! { ":id": id },
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()?;
    Ok(keywords.map(Option::unwrap_or_default))
}

/// The `updated_at` of the oldest selection eviction would consider, so a
/// caller can schedule the next sweep instead of polling.
///
/// `None` means there is nothing evictable.
///
/// # Errors
///
/// Returns [`Error::Database`] if the query fails.
pub fn oldest_evictable(db: &Connection, preserve_tagged: bool) -> Result<Option<i64>> {
    let sql = if preserve_tagged {
        "SELECT MIN(updated_at) FROM selection WHERE pinned_at IS NULL AND keywords == ''"
    } else {
        "SELECT MIN(updated_at) FROM selection"
    };
    // MIN over no rows is one row holding NULL, so "no rows" and "no evictable
    // rows" both arrive as `None`.
    Ok(db.query_row(sql, [], |row| row.get(0))?)
}

/// One offer of a selection, as [`find_selection`] reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OfferRecord {
    /// The offer id, which names its payload on disk.
    pub id: String,
    /// The MIME type it is in.
    pub mime_type: String,
    /// Whether that payload is encrypted.
    pub encryption: EncryptionType,
}

/// A selection and its offers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectionRecord {
    /// The application it came from, if recorded.
    pub source: Option<String>,
    /// Its offers, which may be empty.
    pub offers: Vec<OfferRecord>,
}

/// Find a selection and every offer it has.
///
/// The `LEFT JOIN` is why a selection with no offers is `Some` with an empty
/// `offers` rather than `None`: "no such selection" and "a selection nothing
/// was stored for" are different answers, and the caller needs to tell them
/// apart.
///
/// # Errors
///
/// Returns [`Error::Database`] if the query fails, or
/// [`crate::kind::UnknownDiscriminant`] wrapped in it if a stored
/// `encryption_type` names no variant.
pub fn find_selection(db: &Connection, id: &str) -> Result<Option<SelectionRecord>> {
    let mut stmt = db.prepare_cached(
        "SELECT s.source, o.id, o.mime_type, o.encryption_type \
         FROM selection s \
         LEFT JOIN data_offer o ON o.selection_id = s.id \
         WHERE s.id = :id",
    )?;
    let mut rows = stmt.query(named_params! { ":id": id })?;

    let mut found: Option<SelectionRecord> = None;
    while let Some(row) = rows.next()? {
        let record = match &mut found {
            Some(record) => record,
            None => found.insert(SelectionRecord {
                source: row.get(0)?,
                offers: Vec::new(),
            }),
        };
        let Some(offer_id) = row.get::<_, Option<String>>(1)? else {
            continue;
        };
        record.offers.push(OfferRecord {
            id: offer_id,
            mime_type: row.get::<_, Option<String>>(2)?.unwrap_or_default(),
            encryption: encryption_at(row, 3)?,
        });
    }
    Ok(found)
}

/// The stored `encryption_type` in `column`, refused as a conversion failure
/// when it names no variant.
fn encryption_at(row: &rusqlite::Row<'_>, column: usize) -> rusqlite::Result<EncryptionType> {
    let stored = row.get::<_, Option<i64>>(column)?.unwrap_or(0);
    EncryptionType::from_stored(stored).map_err(|err| {
        rusqlite::Error::FromSqlConversionFailure(
            column,
            rusqlite::types::Type::Integer,
            Box::new(err),
        )
    })
}

/// The offer a selection's list entry shows — the one whose MIME type matches
/// the selection's `preferred_mime_type`.
///
/// # Errors
///
/// Returns [`Error::Database`] if the query fails.
pub fn find_preferred_offer(db: &Connection, selection_id: &str) -> Result<Option<OfferRecord>> {
    let offer = db
        .prepare_cached(
            "SELECT o.id, o.mime_type, o.encryption_type FROM data_offer o \
             JOIN selection s ON s.id = o.selection_id \
             WHERE o.mime_type = s.preferred_mime_type AND o.selection_id = :id",
        )?
        .query_row(named_params! { ":id": selection_id }, |row| {
            Ok(OfferRecord {
                id: row.get::<_, Option<String>>(0)?.unwrap_or_default(),
                mime_type: row.get::<_, Option<String>>(1)?.unwrap_or_default(),
                encryption: encryption_at(row, 2)?,
            })
        })
        .optional()?;
    Ok(offer)
}
