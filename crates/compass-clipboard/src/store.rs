//! Reading clipboard history — the paginated query.
//!
//! Ports `ClipboardDatabase::query`. That function has two SQL shapes and picks
//! between them on whether anything is being filtered, and the difference is not
//! cosmetic:
//!
//! * **Unfiltered** pages `selection` directly, with `COUNT(*) OVER()` computed
//!   inside the subquery so it counts every row rather than the page. SQL
//!   applies `LIMIT` after window functions, which is what makes that work.
//! * **Filtered** joins `selection_fts`, `AND`s the conditions
//!   [`crate::search::plan`] produced, groups by selection, and
//!   wraps the whole thing in `SELECT * FROM (...) LIMIT ? OFFSET ?`.
//!
//! The `GROUP BY` is load-bearing rather than defensive: `selection_fts` holds
//! more than one row per selection — the content, and separately the keywords,
//! which the `selection_auk` trigger maintains — so the join multiplies rows,
//! and without grouping a two-row selection would appear twice and be counted
//! twice.

use compass_sqlcipher_sys::rusqlite::{self, Connection, Row, ToSql};

use crate::kind::{EncryptionType, OfferKind};
use crate::search;

/// One row of clipboard history, as the list view needs it.
///
/// Mirrors `ClipboardHistoryEntry`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryEntry {
    /// The selection id.
    pub id: String,
    /// The preferred offer's MIME type.
    pub mime_type: String,
    /// A short preview of the text, if the offer has one.
    pub text_preview: String,
    /// When it was pinned, or zero if it is not pinned.
    pub pinned_at: i64,
    /// User-supplied keywords indexed alongside the content.
    pub keywords: String,
    /// The offer's MD5.
    pub md5sum: String,
    /// When the selection was last re-selected.
    pub updated_at: i64,
    /// The offer's size in bytes.
    pub size: i64,
    /// What kind of thing it is.
    pub kind: OfferKind,
    /// The host, for link entries, so a favicon can be fetched without parsing.
    pub url_host: Option<String>,
    /// Whether the payload on disk is encrypted.
    pub encryption: EncryptionType,
}

/// What to filter the history by.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ListSettings {
    /// The raw text from the search box.
    pub query: String,
    /// Restrict to one kind.
    pub kind: Option<OfferKind>,
}

/// A page of history, plus what the caller needs to page through it.
///
/// Mirrors `PaginatedResponse`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Page {
    /// The rows on this page.
    pub data: Vec<HistoryEntry>,
    /// How many rows match in total, ignoring `limit` and `offset`.
    pub total_count: i64,
    /// Which page this is.
    pub current_page: i64,
    /// How many pages there are in total.
    pub total_pages: i64,
}

/// Errors from reading history.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// SQLite refused.
    #[error(transparent)]
    Database(#[from] rusqlite::Error),

    /// A stored `kind` or `encryption_type` names no known variant.
    #[error(transparent)]
    UnknownDiscriminant(#[from] crate::kind::UnknownDiscriminant),

    /// `limit` was zero or negative.
    ///
    /// The C++ divides by `limit` to compute `totalPages`, so a zero there is a
    /// division by zero whose result is cast to `int`. Refusing is the fix.
    #[error("limit must be positive, got {0}")]
    NonPositiveLimit(i64),
}

/// The columns both shapes select, in the order both select them.
///
/// Written once because the two queries must agree: the row reader indexes by
/// position, so a column added to one shape and not the other would read the
/// wrong field rather than fail.
const COLUMNS: &str = "\
      s.id, o.mime_type, o.text_preview, s.pinned_at, o.content_hash_md5,
      s.updated_at, o.size, s.kind, o.url_host, o.encryption_type";

/// A text column, empty when NULL.
fn text(row: &Row<'_>, column: usize) -> rusqlite::Result<String> {
    Ok(row.get::<_, Option<String>>(column)?.unwrap_or_default())
}

/// An integer column, zero when NULL, as `QVariant::toLongLong` reads it.
fn int(row: &Row<'_>, column: usize) -> rusqlite::Result<i64> {
    Ok(row.get::<_, Option<i64>>(column)?.unwrap_or(0))
}

fn read_row(row: &Row<'_>) -> Result<(HistoryEntry, i64), Error> {
    let entry = HistoryEntry {
        id: text(row, 0)?,
        mime_type: text(row, 1)?,
        text_preview: text(row, 2)?,
        pinned_at: int(row, 3)?,
        md5sum: text(row, 4)?,
        updated_at: int(row, 5)?,
        size: int(row, 6)?,
        kind: OfferKind::from_stored(int(row, 7)?)?,
        url_host: row.get(8)?,
        encryption: EncryptionType::from_stored(int(row, 9)?)?,
        keywords: text(row, 11)?,
    };
    Ok((entry, int(row, 10)?))
}

/// Read one page of history.
///
/// # Errors
///
/// Returns [`Error::NonPositiveLimit`] for a `limit` of zero or less,
/// [`Error::UnknownDiscriminant`] if a stored enum value names no variant, and
/// [`Error::Database`] if SQLite refuses.
pub fn query(db: &Connection, limit: i64, offset: i64, opts: &ListSettings) -> Result<Page, Error> {
    if limit <= 0 {
        return Err(Error::NonPositiveLimit(limit));
    }

    let plan = search::plan(&opts.query);
    let has_query = !plan.is_empty();
    let has_filters = has_query || opts.kind.is_some();

    let sql = if has_filters {
        filtered_sql(&plan, opts.kind.is_some(), has_query)
    } else {
        unfiltered_sql()
    };

    let kind = opts.kind.map(OfferKind::to_stored);
    let fts_query = plan.match_phrases.join(" AND ");
    let term_names: Vec<String> = (0..plan.short_terms.len())
        .map(|index| format!(":term{index}"))
        .collect();

    let mut params: Vec<(&str, &dyn ToSql)> = Vec::with_capacity(4 + term_names.len());
    params.push((":limit", &limit));
    params.push((":offset", &offset));
    if let Some(kind) = &kind {
        params.push((":kind", kind));
    }
    if !plan.match_phrases.is_empty() {
        params.push((":fts_query", &fts_query));
    }
    for (name, term) in term_names.iter().zip(&plan.short_terms) {
        params.push((name.as_str(), term));
    }

    let mut stmt = db.prepare_cached(&sql)?;
    let mut rows = stmt.query(params.as_slice())?;
    let mut page = Page::default();
    while let Some(row) = rows.next()? {
        let (entry, total) = read_row(row)?;
        page.total_count = total;
        page.data.push(entry);
    }

    page.total_pages = ceil_div(page.total_count, limit);
    page.current_page = ceil_div(offset.max(0), limit);
    Ok(page)
}

/// Ceiling division for non-negative `a` and positive `b`.
///
/// The C++ computes both of these as `ceil(static_cast<double>(x) / limit)`.
/// Doing it in integers avoids the double round trip, and is exact for counts
/// large enough that an `f64` would not be — but the arithmetic is deliberately
/// the same, including `current_page`'s oddity: with an `offset` that is not a
/// multiple of `limit`, ceiling rounds *up*, so offset 50 of limit 100 reports
/// page 1 while showing rows 50..149. Floor would be the conventional answer.
/// It is reproduced rather than fixed because it is a display value that the
/// C++ UI already agrees with, and changing it silently would make the two
/// engines disagree about what page the user is on.
fn ceil_div(a: i64, b: i64) -> i64 {
    debug_assert!(b > 0, "callers check limit");
    if a <= 0 {
        return 0;
    }
    (a - 1) / b + 1
}

/// The shape used when nothing is filtered.
///
/// `COUNT(*) OVER()` sits inside the subquery, where it counts every row of
/// `selection`: SQL evaluates window functions before `LIMIT`, so the count is
/// of the whole table rather than of the page.
fn unfiltered_sql() -> String {
    format!(
        "SELECT
{COLUMNS}, s.total_count, s.keywords
         FROM (
           SELECT rowid AS seq, id, pinned_at, updated_at, kind, preferred_mime_type,
                  keywords, COUNT(*) OVER() AS total_count
           FROM selection
           ORDER BY pinned_at DESC, updated_at DESC, rowid DESC
           LIMIT :limit OFFSET :offset
         ) s
         JOIN data_offer o
           ON o.selection_id = s.id
           AND o.mime_type = s.preferred_mime_type
         ORDER BY s.pinned_at DESC, s.updated_at DESC, s.seq DESC"
    )
}

/// The shape used when anything is filtered.
fn filtered_sql(plan: &search::Plan<'_>, by_kind: bool, has_query: bool) -> String {
    let mut sql = format!(
        "SELECT
{COLUMNS}, COUNT(*) OVER() AS total_count, s.keywords
         FROM selection s
         JOIN data_offer o
           ON o.selection_id = s.id
           AND o.mime_type = s.preferred_mime_type"
    );

    if has_query {
        sql.push_str(" JOIN selection_fts ON selection_fts.selection_id = s.id");
    }

    let mut conditions: Vec<String> = Vec::new();
    if !plan.match_phrases.is_empty() {
        conditions.push("selection_fts MATCH :fts_query".to_owned());
    }
    for index in 0..plan.short_terms.len() {
        conditions.push(format!(
            "instr(lower(selection_fts.content), lower(:term{index})) > 0"
        ));
    }
    if by_kind {
        conditions.push("s.kind = :kind".to_owned());
    }
    if !conditions.is_empty() {
        sql.push_str(" WHERE ");
        sql.push_str(&conditions.join(" AND "));
    }

    // GROUP BY because selection_fts holds a row for the content and another
    // for the keywords, so the join multiplies rows per selection.
    // rowid breaks a tie on updated_at: the later insert is the later copy.
    sql.push_str(" GROUP BY s.id ORDER BY s.pinned_at DESC, s.updated_at DESC, s.rowid DESC");

    format!("SELECT * FROM ({sql}) LIMIT :limit OFFSET :offset")
}
