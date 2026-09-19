//! The Search Files command.
//!
//! Ports `src/server/src/builtins/file/` — when a query is read as a path
//! rather than a search, which of three result modes the view is in, how a
//! late result is discarded, and how the category filter is stored and read
//! back.

use crate::file_category::FileCategory;

/// How many recently-accessed files the empty query shows.
pub const RECENT_FILES_LIMIT: usize = 50;

/// Whether the text looks like a path the user typed on purpose.
///
/// The list is deliberately short and anchored. `~` and `.` and `..` count
/// only when they are the *whole* query, because a search for `..` is not a
/// thing but a search for a file called `notes..txt` is; and the prefixed
/// forms need their separator, so `~notes` stays a search rather than becoming
/// a home-relative path that does not exist.
#[must_use]
pub fn is_explicit_path_query(text: &str) -> bool {
    // The C++ guards the empty string first. Here none of the tests below can
    // match it, so the guard would be unreachable and is left out rather than
    // carried over as a line no mutation can reach.
    text.starts_with('/')
        || text == "~"
        || text.starts_with("~/")
        || text == "."
        || text == ".."
        || text.starts_with("./")
        || text.starts_with("../")
}

/// The same test with the Windows spellings added.
///
/// A backslash *anywhere* makes it a path there, not only at the front, and a
/// drive letter counts when followed by either separator.
#[must_use]
pub fn is_explicit_path_query_windows(text: &str) -> bool {
    if is_explicit_path_query(text) {
        return true;
    }
    if text.contains('\\') {
        return true;
    }
    let chars: Vec<char> = text.chars().take(3).collect();
    chars.len() >= 3 && chars[1] == ':' && (chars[2] == '/' || chars[2] == '\\')
}

/// Which of three ways the list is being filled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResultMode {
    /// The query named one file and that file is being shown.
    DirectPath,
    /// The empty query is showing recently-accessed files.
    Recent,
    /// The index is being searched.
    IndexedSearch,
}

/// What the view decided to do with the text that was typed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueryPlan {
    /// Show recently-accessed files, or fall through to an empty search.
    EmptyQuery,
    /// Show this one path under the "Direct file path" heading.
    DirectPath(String),
    /// The query named a real path, but the category filter excludes it. The
    /// heading still says "Direct file path" and the section is empty.
    DirectPathFiltered,
    /// Search the index, after the indexer's own debounce.
    Search,
}

/// Decide what to do with a query.
///
/// The direct-path branch needs three things at once: the text has to look
/// like a path, the path has to exist, and it must not be `/` — the root
/// exists on every machine and matching it would turn a single slash into a
/// one-item list instead of a search for files with a slash in their name.
///
/// When a category filter is on and the named file is the wrong kind, the
/// result is an *empty* direct-path section rather than a fall-through to the
/// index: the user asked for that file, and answering with a list of other
/// files would be a different question.
#[must_use]
pub fn plan_query(
    text: &str,
    expanded: &str,
    exists: bool,
    is_path_query: bool,
    category_of_path: Option<FileCategory>,
    selected_category: Option<FileCategory>,
) -> QueryPlan {
    if text.trim().is_empty() {
        return QueryPlan::EmptyQuery;
    }
    if is_path_query && expanded != "/" && exists {
        if let Some(wanted) = selected_category
            && category_of_path != Some(wanted)
        {
            return QueryPlan::DirectPathFiltered;
        }
        return QueryPlan::DirectPath(expanded.to_owned());
    }
    QueryPlan::Search
}

/// What the empty query does first.
///
/// Recently-accessed files when the system can supply them, and the index
/// otherwise. A machine with no recent-files record is not shown an empty list
/// — it falls straight through to the index.
#[must_use]
pub fn empty_query_mode(recent_available: bool) -> ResultMode {
    if recent_available {
        ResultMode::Recent
    } else {
        ResultMode::IndexedSearch
    }
}

/// Whether a returned batch of recent files is worth showing.
///
/// An empty answer falls through to the index rather than leaving the list
/// blank — a system that tracks recent files but has none yet should still
/// show something.
#[must_use]
pub fn recent_files_usable(count: usize) -> bool {
    count > 0
}

/// Whether a result that has arrived still belongs on screen.
///
/// Three conditions, and all three are needed. The task may have been
/// cancelled; the view may have changed mode since (a direct path typed while
/// a search was in flight); and the query may have moved on, which is the one
/// that matters most — results arriving out of order would otherwise leave the
/// list showing an answer to a question the user has already finished asking.
#[must_use]
pub fn result_is_current(
    cancelled: bool,
    mode: ResultMode,
    expected_mode: ResultMode,
    query_at_request: &str,
    query_now: &str,
) -> bool {
    !cancelled && mode == expected_mode && query_at_request == query_now
}

/// Whether a recent-files answer still belongs on screen.
///
/// The query is not compared to a remembered one here: recent files are only
/// ever requested for the empty query, so the test is that the box is *still*
/// empty.
#[must_use]
pub fn recent_result_is_current(cancelled: bool, mode: ResultMode, search_text: &str) -> bool {
    !cancelled && mode == ResultMode::Recent && search_text.is_empty()
}

/// The heading over a set of search results.
#[must_use]
pub fn results_heading(query: &str) -> &'static str {
    if query.is_empty() {
        "Recently Modified"
    } else {
        "Results"
    }
}

/// The heading over recently-accessed files.
pub const RECENT_HEADING: &str = "Recently Accessed";

/// The heading over a directly-named path.
pub const DIRECT_PATH_HEADING: &str = "Direct file path";

/// The category filter's options, in order.
///
/// Index 0 is `All`, which is not a category but the absence of one.
pub const CATEGORY_FILTER_KEYS: &[&str] = &[
    "All",
    "Other",
    "Directories",
    "Images",
    "Videos",
    "Audio",
    "Documents",
    "Archives",
    "Applications",
];

/// The key the chosen filter is stored under.
pub const CATEGORY_STORAGE_KEY: &str = "fileCategory";

/// The category a filter index selects, or `None` for `All`.
///
/// Anything outside the list is `None` too, so a stored index from a future
/// version's longer list shows everything rather than nothing.
#[must_use]
pub fn category_for_index(index: usize) -> Option<FileCategory> {
    match index {
        1 => Some(FileCategory::Other),
        2 => Some(FileCategory::Directory),
        3 => Some(FileCategory::Image),
        4 => Some(FileCategory::Video),
        5 => Some(FileCategory::Audio),
        6 => Some(FileCategory::Document),
        7 => Some(FileCategory::Archive),
        8 => Some(FileCategory::Application),
        _ => None,
    }
}

/// Whether a chosen index is worth acting on.
///
/// Out of range is refused, and so is the index already selected — reselecting
/// the current filter would otherwise re-run the search for no reason.
///
/// The `index >= 0` half is carried over from the C++ and is redundant here:
/// the cast to `usize` turns a negative into a very large number, which the
/// upper bound already rejects. It is kept because it says what is meant, and
/// because it is what still holds if the bound is ever compared signed.
#[must_use]
pub fn accepts_filter_change(index: i32, current: i32) -> bool {
    index >= 0 && (index as usize) < CATEGORY_FILTER_KEYS.len() && index != current
}

/// The value written to storage for a filter index.
///
/// The **untranslated** key is stored, not the label the user sees. Storing
/// the label would mean a filter chosen in one language is unreadable in
/// another.
#[must_use]
pub fn stored_key_for_index(index: usize) -> &'static str {
    CATEGORY_FILTER_KEYS.get(index).copied().unwrap_or_default()
}

/// The filter index a stored value restores, if any.
///
/// A missing value, an unknown one, and `All` all restore nothing — `All` is
/// already the default, so writing it back would be a no-op with a signal
/// emitted. That is what the C++'s `index <= 0` means, and it folds the
/// not-found case (`-1`) into the same branch.
#[must_use]
pub fn restored_filter_index(stored: Option<&str>) -> Option<usize> {
    let stored = stored?;
    let index = CATEGORY_FILTER_KEYS.iter().position(|k| *k == stored)?;
    if index == 0 {
        return None;
    }
    Some(index)
}

/// Whether a debounce of this length is worth waiting out.
///
/// A zero debounce runs the search immediately rather than starting a timer
/// that fires on the next tick — which is what makes a fast local index feel
/// like typing into a list rather than into a search box.
#[must_use]
pub fn should_debounce(debounce_ms: u64) -> bool {
    debounce_ms != 0
}

/// The commands the file extension registers.
///
/// Rebuilding the index is written and deliberately **not** registered: the
/// indexer now sweeps on a timer, and the C++ leaves the command in place with
/// a comment saying the same effect is had by deleting the indexer's cache
/// directory. The port keeps it unregistered for the same reason.
#[must_use]
pub fn registered_commands() -> Vec<&'static str> {
    vec!["search"]
}

/// Whether Search Files answers a query no other command claimed.
///
/// It is a fallback command, so a search that matches no command name still
/// searches the filesystem instead of coming back empty.
pub const SEARCH_IS_FALLBACK: bool = true;
