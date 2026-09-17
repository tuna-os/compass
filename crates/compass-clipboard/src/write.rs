//! Writing clipboard history — inserts, indexing, deletion, and bubbling up.
//!
//! Ports the write half of `ClipboardDatabase`. Two of these fix bugs rather
//! than reproduce them; both are recorded in
//! [PARITY.md](../../../docs/rust-engine/PARITY.md) and both have a test.

use compass_sqlcipher_sys::{Database, Transaction};

use crate::kind::{EncryptionType, OfferKind};

/// How much of a selection's text is indexed for search.
///
/// `MAX_INDEXED_CONTENT_SIZE` in `clipboard-db.cpp`, where it is applied as
/// `content.left(...)`. `QString::left` counts **UTF-16 code units**, not
/// characters and not bytes, so [`index_content`] truncates the same way — see
/// its docs for the one place this cannot follow exactly.
pub const MAX_INDEXED_CONTENT: usize = 1 << 16;

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
    Database(#[from] compass_sqlcipher_sys::Error),
}

type Result<T> = std::result::Result<T, Error>;

/// Now, in whole seconds since the epoch.
///
/// Taken once per call and bound, rather than left to SQLite's `unixepoch()`.
/// That is not a style preference: see [`evict_older_than`].
fn now() -> i64 {
    i64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
    )
    .unwrap_or(i64::MAX)
}

/// Record a new selection. `created_at` and `updated_at` are both set to now.
///
/// # Errors
///
/// Returns [`Error::Database`] if the insert fails — including on a duplicate
/// id, which the primary key rejects.
pub fn insert_selection(db: &Database, selection: &NewSelection<'_>) -> Result<()> {
    let mut stmt = db.prepare(
        "INSERT INTO selection (id, kind, offer_count, hash_md5, preferred_mime_type, source, \
         created_at, updated_at) \
         VALUES (:id, :kind, :offer_count, :hash_md5, :preferred_mime_type, :source, :epoch, :epoch)",
    )?;
    stmt.bind_text(":id", selection.id)?;
    stmt.bind_int64(":kind", selection.kind.to_stored())?;
    stmt.bind_int64(":offer_count", selection.offer_count)?;
    stmt.bind_text(":hash_md5", selection.hash)?;
    stmt.bind_text(":preferred_mime_type", selection.preferred_mime_type)?;
    match selection.source {
        Some(source) => stmt.bind_text(":source", source)?,
        None => stmt.bind_null(":source")?,
    }
    stmt.bind_int64(":epoch", now())?;
    stmt.step()?;
    Ok(())
}

/// Record one offer of a selection.
///
/// # Errors
///
/// Returns [`Error::Database`] if the insert fails.
pub fn insert_offer(db: &Database, offer: &NewOffer<'_>) -> Result<()> {
    let mut stmt = db.prepare(
        "INSERT INTO data_offer (id, selection_id, mime_type, text_preview, content_hash_md5, \
         encryption_type, size, kind, url_host) \
         VALUES (:id, :selection_id, :mime_type, :text_preview, :content_hash_md5, :encryption, \
         :size, :kind, :url_host)",
    )?;
    stmt.bind_text(":id", offer.id)?;
    stmt.bind_text(":selection_id", offer.selection_id)?;
    stmt.bind_text(":mime_type", offer.mime_type)?;
    stmt.bind_text(":text_preview", offer.text_preview)?;
    stmt.bind_text(":content_hash_md5", offer.md5sum)?;
    stmt.bind_int64(":encryption", offer.encryption.to_stored())?;
    stmt.bind_int64(":size", offer.size)?;
    stmt.bind_int64(":kind", offer.kind.to_stored())?;
    match offer.url_host {
        Some(host) => stmt.bind_text(":url_host", host)?,
        None => stmt.bind_null(":url_host")?,
    }
    stmt.step()?;
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
pub fn index_content(db: &Database, selection_id: &str, content: &str) -> Result<()> {
    let mut stmt =
        db.prepare("INSERT INTO selection_fts (selection_id, content) VALUES (:id, :content)")?;
    stmt.bind_text(":id", selection_id)?;
    stmt.bind_text(":content", truncate_for_index(content))?;
    stmt.step()?;
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
/// row — per statement rather than per connection. `compass-sqlcipher-sys`
/// deliberately does not expose `sqlite3_changes`, so the original shape is not
/// available to reproduce by accident.
///
/// # Errors
///
/// Returns [`Error::Database`] if the update fails, rather than reporting a
/// stale success.
pub fn bubble_up(db: &Database, id_or_hash: &str) -> Result<bool> {
    let mut stmt = db.prepare(
        "UPDATE selection SET updated_at = :updated_at \
         WHERE hash_md5 = :id OR id = :id RETURNING id",
    )?;
    stmt.bind_text(":id", id_or_hash)?;
    stmt.bind_int64(":updated_at", now())?;
    stmt.step().map_err(Error::Database)
}

/// Delete a selection and its offers, returning the offer ids whose payloads
/// the caller must now unlink from disk.
///
/// # Errors
///
/// Returns [`Error::Database`] if either delete fails, in which case nothing is
/// committed.
pub fn remove_selection(db: &Database, selection_id: &str) -> Result<Vec<String>> {
    let tx = db.transaction()?;
    let removed = delete_offers_of(db, selection_id)?;

    let mut stmt = db.prepare("DELETE FROM selection WHERE id = :id")?;
    stmt.bind_text(":id", selection_id)?;
    stmt.step()?;
    drop(stmt);

    tx.commit()?;
    Ok(removed)
}

fn delete_offers_of(db: &Database, selection_id: &str) -> Result<Vec<String>> {
    let mut stmt = db.prepare("DELETE FROM data_offer WHERE selection_id = :id RETURNING id")?;
    stmt.bind_text(":id", selection_id)?;
    let mut removed = Vec::new();
    while stmt.step()? {
        if let Some(id) = stmt.column_text(0) {
            removed.push(id);
        }
    }
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
    db: &Database,
    age: std::time::Duration,
    preserve_tagged: bool,
) -> Result<Vec<String>> {
    const PRESERVE: &str = " AND pinned_at IS NULL AND keywords == ''";

    // One reading of the clock, used by both statements below.
    let cutoff = now().saturating_sub(i64::try_from(age.as_secs()).unwrap_or(i64::MAX));

    let tx = db.transaction()?;

    let mut select = String::from(
        "SELECT o.id FROM data_offer o \
         JOIN selection s ON s.id = o.selection_id \
         WHERE s.updated_at < :cutoff",
    );
    if preserve_tagged {
        select.push_str(PRESERVE);
    }
    let mut stmt = db.prepare(&select)?;
    stmt.bind_int64(":cutoff", cutoff)?;
    let mut evicted = Vec::new();
    while stmt.step()? {
        if let Some(id) = stmt.column_text(0) {
            evicted.push(id);
        }
    }
    drop(stmt);

    if evicted.is_empty() {
        return Ok(evicted);
    }

    let mut delete = String::from("DELETE FROM selection WHERE updated_at < :cutoff");
    if preserve_tagged {
        delete.push_str(PRESERVE);
    }
    let mut stmt = db.prepare(&delete)?;
    stmt.bind_int64(":cutoff", cutoff)?;
    stmt.step()?;
    drop(stmt);

    tx.commit()?;
    Ok(evicted)
}

/// Delete every selection, returning the offer ids to unlink.
///
/// # Errors
///
/// Returns [`Error::Database`] if either statement fails.
pub fn remove_all(db: &Database, preserve_tagged: bool) -> Result<Vec<String>> {
    let tx = db.transaction()?;

    let (select, delete) = if preserve_tagged {
        (
            "SELECT o.id FROM data_offer o JOIN selection s ON s.id = o.selection_id \
             WHERE s.pinned_at IS NULL AND s.keywords == ''",
            "DELETE FROM selection WHERE pinned_at IS NULL AND keywords == ''",
        )
    } else {
        ("SELECT id FROM data_offer", "DELETE FROM selection")
    };

    let mut stmt = db.prepare(select)?;
    let mut removed = Vec::new();
    while stmt.step()? {
        if let Some(id) = stmt.column_text(0) {
            removed.push(id);
        }
    }
    drop(stmt);

    let mut stmt = db.prepare(delete)?;
    stmt.step()?;
    drop(stmt);

    tx.commit()?;
    Ok(removed)
}

/// Borrow of a transaction so callers can group several writes.
///
/// Re-exported so a caller does not have to depend on `compass-sqlcipher-sys`
/// directly to batch an insert of a selection and its offers.
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
pub fn set_pinned(db: &Database, id: &str, pinned: bool) -> Result<bool> {
    let mut stmt = if pinned {
        let mut stmt =
            db.prepare("UPDATE selection SET pinned_at = :epoch WHERE id = :id RETURNING id")?;
        stmt.bind_int64(":epoch", now())?;
        stmt
    } else {
        db.prepare("UPDATE selection SET pinned_at = NULL WHERE id = :id RETURNING id")?
    };
    stmt.bind_text(":id", id)?;
    stmt.step().map_err(Error::Database)
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
pub fn set_keywords(db: &Database, id: &str, keywords: &str) -> Result<bool> {
    let mut stmt = db.prepare("UPDATE selection SET keywords = :kw WHERE id = :id RETURNING id")?;
    stmt.bind_text(":kw", keywords)?;
    stmt.bind_text(":id", id)?;
    stmt.step().map_err(Error::Database)
}

/// A selection's keywords, or `None` if there is no such selection.
///
/// A selection that exists with no keywords reads back as `Some("")`, because
/// the column defaults to the empty string rather than to NULL.
///
/// # Errors
///
/// Returns [`Error::Database`] if the query fails.
pub fn keywords_of(db: &Database, id: &str) -> Result<Option<String>> {
    let mut stmt = db.prepare("SELECT keywords FROM selection WHERE id = :id")?;
    stmt.bind_text(":id", id)?;
    if stmt.step()? {
        Ok(Some(stmt.column_text(0).unwrap_or_default()))
    } else {
        Ok(None)
    }
}

/// The `updated_at` of the oldest selection eviction would consider, so a
/// caller can schedule the next sweep instead of polling.
///
/// `None` means there is nothing evictable.
///
/// # Errors
///
/// Returns [`Error::Database`] if the query fails.
pub fn oldest_evictable(db: &Database, preserve_tagged: bool) -> Result<Option<i64>> {
    let sql = if preserve_tagged {
        "SELECT MIN(updated_at) FROM selection WHERE pinned_at IS NULL AND keywords == ''"
    } else {
        "SELECT MIN(updated_at) FROM selection"
    };
    let mut stmt = db.prepare(sql)?;
    if stmt.step()? && !stmt.is_null(0) {
        Ok(Some(stmt.column_int64(0)))
    } else {
        // MIN over no rows is one row holding NULL, so "no rows" and "no
        // evictable rows" both arrive here.
        Ok(None)
    }
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
pub fn find_selection(db: &Database, id: &str) -> Result<Option<SelectionRecord>> {
    let mut stmt = db.prepare(
        "SELECT s.source, o.id, o.mime_type, o.encryption_type \
         FROM selection s \
         LEFT JOIN data_offer o ON o.selection_id = s.id \
         WHERE s.id = :id",
    )?;
    stmt.bind_text(":id", id)?;

    let mut found: Option<SelectionRecord> = None;
    while stmt.step()? {
        let record = found.get_or_insert_with(|| SelectionRecord {
            source: stmt.column_text(0),
            offers: Vec::new(),
        });
        if stmt.is_null(1) {
            continue;
        }
        record.offers.push(OfferRecord {
            id: stmt.column_text(1).unwrap_or_default(),
            mime_type: stmt.column_text(2).unwrap_or_default(),
            encryption: EncryptionType::from_stored(stmt.column_int64(3)).map_err(|err| {
                Error::Database(compass_sqlcipher_sys::Error::Sqlite {
                    context: "reading an offer's encryption type",
                    message: err.to_string(),
                    code: -1,
                })
            })?,
        });
    }
    Ok(found)
}

/// The offer a selection's list entry shows — the one whose MIME type matches
/// the selection's `preferred_mime_type`.
///
/// # Errors
///
/// Returns [`Error::Database`] if the query fails.
pub fn find_preferred_offer(db: &Database, selection_id: &str) -> Result<Option<OfferRecord>> {
    let mut stmt = db.prepare(
        "SELECT o.id, o.mime_type, o.encryption_type FROM data_offer o \
         JOIN selection s ON s.id = o.selection_id \
         WHERE o.mime_type = s.preferred_mime_type AND o.selection_id = :id",
    )?;
    stmt.bind_text(":id", selection_id)?;
    if !stmt.step()? {
        return Ok(None);
    }
    Ok(Some(OfferRecord {
        id: stmt.column_text(0).unwrap_or_default(),
        mime_type: stmt.column_text(1).unwrap_or_default(),
        encryption: EncryptionType::from_stored(stmt.column_int64(2)).map_err(|err| {
            Error::Database(compass_sqlcipher_sys::Error::Sqlite {
                context: "reading the preferred offer's encryption type",
                message: err.to_string(),
                code: -1,
            })
        })?,
    }))
}
