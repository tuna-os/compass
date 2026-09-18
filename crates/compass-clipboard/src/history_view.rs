//! The clipboard history command's own decisions.
//!
//! Ports `src/server/src/builtins/clipboard/` — which icon a stored entry
//! shows, what its action panel offers and in what order, how the kind filter
//! is stored and read back, and the query controller's single-flight rule.

use crate::kind::{EncryptionType, OfferKind};

/// Which of copy and paste the return key runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DefaultAction {
    /// Put it on the clipboard and leave it there.
    #[default]
    Copy,
    /// Put it on the clipboard and paste it into whatever had focus.
    Paste,
}

impl DefaultAction {
    /// Read the stored preference.
    ///
    /// Only the exact string `paste` selects pasting; anything else — an
    /// absent preference, an empty one, a spelling from a future version —
    /// falls back to copying. That is the safer of the two to get wrong:
    /// copying into the wrong window does nothing, pasting into it does not.
    #[must_use]
    pub fn parse(preference: Option<&str>) -> Self {
        if preference == Some("paste") {
            Self::Paste
        } else {
            Self::Copy
        }
    }
}

/// The icon a stored entry shows.
///
/// A link shows the site's favicon where the host is known, falling back to
/// the generic link glyph — so a list of links is scannable by site rather
/// than being a column of identical chain icons. Everything the enum does not
/// name shows a question mark rather than nothing, which is how a row written
/// by a newer build still occupies a visible line.
#[must_use]
pub fn entry_icon(kind: OfferKind, url_host: Option<&str>) -> EntryIcon {
    match kind {
        OfferKind::Image => EntryIcon::Builtin("image"),
        OfferKind::Link => match url_host {
            Some(host) => EntryIcon::Favicon {
                host: host.to_owned(),
                fallback: "link",
            },
            None => EntryIcon::Builtin("link"),
        },
        OfferKind::Text => EntryIcon::Builtin("text"),
        OfferKind::File => EntryIcon::Builtin("folder"),
        _ => EntryIcon::Builtin("question-mark-circle"),
    }
}

/// What [`entry_icon`] chose.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EntryIcon {
    /// A glyph from the built-in set.
    Builtin(&'static str),
    /// A site's icon, with a glyph behind it.
    Favicon {
        /// The host to fetch it from.
        host: String,
        /// What to draw if it cannot be fetched.
        fallback: &'static str,
    },
}

/// Whether an entry's contents can be put back on the clipboard.
///
/// An unencrypted entry always can. An encrypted one can only once the key is
/// available — which it is not before the keyring has been unlocked — so the
/// panel offers a way into the settings instead of a copy action that would
/// fail.
#[must_use]
pub fn is_copyable(encryption: EncryptionType, encryption_ready: bool) -> bool {
    encryption == EncryptionType::None || encryption_ready
}

/// The first section of an entry's action panel.
///
/// Copy and paste are both offered where pasting is supported, in the order
/// the preference asks for — the first is what the return key runs. Where it
/// is not supported only copying appears, rather than a paste action that
/// would do nothing.
///
/// An entry that cannot be copied leads with a way into the settings, and
/// offers neither, because both would fail.
#[must_use]
pub fn main_actions(
    copyable: bool,
    supports_paste: bool,
    default_action: DefaultAction,
) -> Vec<&'static str> {
    if !copyable {
        return vec!["open-settings"];
    }
    if !supports_paste {
        return vec!["copy"];
    }
    match default_action {
        DefaultAction::Copy => vec!["copy", "paste"],
        DefaultAction::Paste => vec!["paste", "copy"],
    }
}

/// The open actions offered for a file entry.
///
/// The stored payload is a URI list, and open actions appear only when it
/// holds **exactly one** entry that exists on disk. A copy of three files has
/// no single thing to open, and a copy of one file that has since been deleted
/// would offer to open nothing.
///
/// `openWith` appears whenever the file exists; `open` needs a default
/// application as well, so a file type nothing claims still gets a chooser.
#[must_use]
pub fn file_open_actions(
    uri_list: &str,
    exists: impl Fn(&str) -> bool,
    has_default: bool,
) -> Vec<&'static str> {
    let urls: Vec<&str> = uri_list
        .split("\r\n")
        .filter(|part| !part.is_empty())
        .collect();
    if urls.len() != 1 || !exists(urls[0]) {
        return Vec::new();
    }
    let mut out = Vec::new();
    if has_default {
        out.push("open");
    }
    out.push("open-with");
    out
}

/// The open actions offered for a link entry.
///
/// A link needs no existence check — the payload *is* the target — so the
/// chooser is always offered and `open` follows the default browser.
#[must_use]
pub fn link_open_actions(has_default: bool) -> Vec<&'static str> {
    let mut out = Vec::new();
    if has_default {
        out.push("open");
    }
    out.push("open-with");
    out
}

/// The sections after the main one.
///
/// Pinning and editing keywords are changes to an entry; removing it and
/// removing everything are not. They are separated so the section break sits
/// between the reversible and the irreversible.
#[must_use]
pub fn trailing_sections() -> Vec<Vec<&'static str>> {
    vec![vec!["pin", "edit-keywords"], vec!["remove", "remove-all"]]
}

/// Whether pinning this entry pins or unpins it.
#[must_use]
pub const fn pin_action_pins(pinned_at: Option<i64>) -> bool {
    pinned_at.is_none()
}

/// The kind filter's options, in order. Index 0 is not a kind.
pub const KIND_FILTER_OPTIONS: &[&str] = &["All", "Text", "Images", "Links", "Files"];

/// The value each filter index is stored as.
///
/// Stored untranslated and in the singular, which is not what the options say:
/// the third option reads `Images` and stores `image`. The stored vocabulary is
/// the enum's, not the interface's, and keeping them apart is what lets either
/// change without the other.
pub const FILTER_STORED_VALUES: &[&str] = &["all", "text", "image", "link", "file"];

/// The key the filter is stored under.
pub const FILTER_STORAGE_KEY: &str = "filter";

/// The kind a stored filter value selects.
#[must_use]
pub fn kind_for_stored_value(value: &str) -> Option<OfferKind> {
    match value {
        "text" => Some(OfferKind::Text),
        "image" => Some(OfferKind::Image),
        "link" => Some(OfferKind::Link),
        "file" => Some(OfferKind::File),
        _ => None,
    }
}

/// The filter index a kind sits at.
///
/// `Unknown` and the enum's count sentinel both answer 0, the same as no
/// filter — there is no option for them, and answering with an out-of-range
/// index would select nothing at all.
#[must_use]
pub fn filter_index_for_kind(kind: Option<OfferKind>) -> usize {
    match kind {
        Some(OfferKind::Text) => 1,
        Some(OfferKind::Image) => 2,
        Some(OfferKind::Link) => 3,
        Some(OfferKind::File) => 4,
        _ => 0,
    }
}

/// The value stored for a filter index.
#[must_use]
pub fn stored_value_for_index(index: usize) -> &'static str {
    FILTER_STORED_VALUES.get(index).copied().unwrap_or("all")
}

/// The filter restored from storage, as (index, kind).
///
/// A missing value reads as `all`, and so does an unrecognised one — neither
/// is a reason to show an empty list.
#[must_use]
pub fn restored_filter(stored: Option<&str>) -> (usize, Option<OfferKind>) {
    let kind = kind_for_stored_value(stored.unwrap_or("all"));
    (filter_index_for_kind(kind), kind)
}

/// The query controller's single-flight state.
///
/// At most one query runs, and at most one waits. A third request while one is
/// running collapses into the same waiting slot rather than queueing, because
/// only the newest matters — and a result is never delivered while something
/// newer is already wanted, since each delivery consumes the view's one-shot
/// "select the first row" flag and a stale one would move the selection under
/// the user.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct QueryFlight {
    /// Whether a query is in flight.
    pub running: bool,
    /// Whether another is wanted once it lands.
    pub pending: bool,
}

/// What a request should do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlightAction {
    /// Send it now.
    Run,
    /// Note that another is wanted.
    Queue,
    /// Deliver the result that just arrived.
    Deliver,
}

impl QueryFlight {
    /// Ask for a query.
    #[must_use]
    pub fn request(self) -> (Self, FlightAction) {
        if self.running {
            return (
                Self {
                    running: true,
                    pending: true,
                },
                FlightAction::Queue,
            );
        }
        (
            Self {
                running: true,
                pending: false,
            },
            FlightAction::Run,
        )
    }

    /// A query has finished.
    ///
    /// A queued request is run instead of delivering what arrived, and the
    /// result that would have been shown is dropped without ever reaching the
    /// list.
    #[must_use]
    pub fn finish(self) -> (Self, FlightAction) {
        if self.pending {
            return (
                Self {
                    running: true,
                    pending: false,
                },
                FlightAction::Run,
            );
        }
        (
            Self {
                running: false,
                pending: false,
            },
            FlightAction::Deliver,
        )
    }
}

/// Whether a delivery should keep the current selection rather than jump to
/// the top.
///
/// The first delivery after the view resets selects the first row; every later
/// one is an *incremental* update — a pin, a rename, a new copy arriving while
/// the user reads — and moving the selection then would take the row out from
/// under them.
#[must_use]
pub const fn delivery_is_incremental(select_first_on_reset: bool) -> bool {
    !select_first_on_reset
}
