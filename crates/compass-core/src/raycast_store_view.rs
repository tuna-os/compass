//! The Raycast extension store's list and detail views.
//!
//! Ports `src/server/src/builtins/raycast/`. The store's API client is
//! elsewhere ([`crate::raycast_store`]); this is what the two views do with
//! what it returns — when a list may be drawn, what each row's compatibility
//! badge says, and what the detail page's banner reads.

use crate::raycast_store::CompatInfo;

/// How well an extension is expected to work here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompatTier {
    /// Everything it uses is supported.
    Compatible,
    /// It runs, with known gaps.
    Partial,
    /// It cannot work.
    Incompatible,
    /// Nobody has checked.
    Unknown,
}

impl CompatTier {
    /// The integer the view model exposes, matching the C++ enum's order.
    #[must_use]
    pub const fn as_i32(self) -> i32 {
        match self {
            Self::Compatible => 0,
            Self::Partial => 1,
            Self::Incompatible => 2,
            Self::Unknown => 3,
        }
    }
}

/// Read a tier out of a compatibility sheet entry.
///
/// Three statuses are recognised and everything else is `Unknown` — including
/// a status the sheet gains later, which is the point: an unrecognised status
/// says "nobody has checked *here*" rather than crashing or claiming
/// compatibility nobody asserted.
#[must_use]
pub fn compat_tier_from_info(info: &CompatInfo) -> CompatTier {
    match info.status.as_str() {
        "full" => CompatTier::Compatible,
        "partial" => CompatTier::Partial,
        "impossible" => CompatTier::Incompatible,
        _ => CompatTier::Unknown,
    }
}

/// The tier shown for an extension, given whether this platform has a sheet.
///
/// On a platform with no compatibility sheet there is no badge at all, which
/// is not the same as an `Unknown` badge: one says the question does not
/// apply, the other says it was asked and not answered. The view model
/// distinguishes them by sending `-1`.
#[must_use]
pub fn row_compat_tier(has_sheet: bool, info: Option<&CompatInfo>) -> Option<CompatTier> {
    if !has_sheet {
        return None;
    }
    Some(info.map_or(CompatTier::Unknown, compat_tier_from_info))
}

/// What the view model sends for a row's tier.
#[must_use]
pub fn row_compat_value(tier: Option<CompatTier>) -> i32 {
    tier.map_or(-1, CompatTier::as_i32)
}

/// The banner shown at the top of an extension's detail page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Alert {
    /// One of `success`, `warning`, `danger`, `muted`.
    pub kind: &'static str,
    /// The sentence shown.
    pub message: String,
    /// The sheet's notes, when it has any.
    pub notes: Vec<String>,
}

/// Build the detail page's compatibility banner.
///
/// An extension the sheet does not mention gets a *different* sentence from
/// one the sheet mentions with an unknown status — "may or may not work" as
/// against "no data is available". Both are muted, so they look the same; they
/// read differently because they mean different things, and only the first can
/// be resolved by someone adding a row to the sheet.
#[must_use]
pub fn build_alert(info: Option<&CompatInfo>) -> Alert {
    let Some(info) = info else {
        return Alert {
            kind: "muted",
            message: "No compatibility data is available — this extension may or may not work."
                .to_owned(),
            notes: Vec::new(),
        };
    };

    let (kind, message) = match compat_tier_from_info(info) {
        CompatTier::Compatible => ("success", "This extension should be fully compatible."),
        CompatTier::Partial => ("warning", "This extension works but has a few quirks."),
        CompatTier::Incompatible => ("danger", "This extension is not compatible."),
        CompatTier::Unknown => (
            "muted",
            "No compatibility data is available for this extension.",
        ),
    };

    Alert {
        kind,
        message: message.to_owned(),
        notes: info.notes.clone().unwrap_or_default(),
    }
}

/// Whether the list may be drawn yet.
///
/// The page and the compatibility sheet are fetched independently and may
/// arrive in either order, so this is a rendezvous rather than a sequence:
/// whichever finishes second is what draws the list. Drawing on the page alone
/// would show every row with no badge and then flicker when the sheet landed.
#[must_use]
pub fn can_populate(compat_ready: bool, page_pending: bool) -> bool {
    compat_ready && page_pending
}

/// Whether a finished list fetch still belongs on screen.
///
/// The list is only ever shown for the empty query, so the test is that the
/// box is *still* empty — not that it matches a remembered string, which is
/// what the search path needs instead.
#[must_use]
pub fn list_result_is_current(search_text: &str) -> bool {
    search_text.is_empty()
}

/// Whether a finished search still belongs on screen.
#[must_use]
pub fn query_result_is_current(query_at_request: &str, query_now: &str) -> bool {
    query_at_request == query_now
}

/// Whether typing this text starts a search or reloads the list.
///
/// Emptying the box reloads the list **immediately**, with no debounce: the
/// answer is already cached, and waiting 200 ms to show something the user has
/// just returned to would be a pause with nothing behind it.
#[must_use]
pub fn text_starts_search(text: &str) -> bool {
    !text.is_empty()
}

/// How long typing settles before a search is sent.
pub const SEARCH_DEBOUNCE_MS: u64 = 200;

/// The heading over the browsable list.
pub const LIST_HEADING: &str = "Extensions";

/// The heading over search results.
pub const SEARCH_HEADING: &str = "Results";

/// Whether a failed fetch clears the loading state.
///
/// It does not. Both handlers report the failure and return *before* clearing
/// it, so a failed fetch leaves the spinner running under the error toast.
/// Ported as it is and pinned by a test: this is a visible behaviour, and
/// changing it here would make the two implementations disagree while the C++
/// is still the one shipping.
pub const FAILED_FETCH_CLEARS_LOADING: bool = false;

/// The message shown when one extension cannot be loaded by name.
///
/// The identifier is rebuilt as `author/name` and quoted, because this view can
/// be reached from a deep link where the user never saw a list — the name they
/// typed is the only context they have.
#[must_use]
pub fn extension_load_failure(author_handle: &str, extension_name: &str) -> (String, String) {
    (
        "Failed to load extension".to_owned(),
        format!(
            "The extension \"{author_handle}/{extension_name}\" could not be loaded. \
             It may not exist or the store may be unreachable."
        ),
    )
}

/// The detail page's navigation title.
#[must_use]
pub fn detail_navigation_title(extension_title: &str) -> String {
    format!("Extension Store - {extension_title}")
}

/// The navigation title before an extension has loaded.
pub const INITIAL_NAVIGATION_TITLE: &str = "Extension Store";

/// The actions on a row in the list.
///
/// Uninstalling sits in a section of its own, so the separator is what stands
/// between it and the primary action.
#[must_use]
pub fn row_actions() -> Vec<Vec<&'static str>> {
    vec![vec!["show-details"], vec!["uninstall"]]
}

/// The actions on the detail page.
///
/// Installing and uninstalling are the same slot, never both — an extension is
/// one or the other. Reporting an issue is always there, because an extension
/// that will not install is exactly the one worth reporting.
#[must_use]
pub fn detail_actions(is_installed: bool) -> Vec<&'static str> {
    vec![
        if is_installed { "uninstall" } else { "install" },
        "report-issue",
    ]
}

/// Whether the detail page has screenshots to show.
///
/// The count comes from the listing's `metadata_count`, not from the
/// screenshot list — the URLs are derived, so the count is what says whether
/// deriving them is worth doing.
#[must_use]
pub const fn has_screenshots(metadata_count: u32) -> bool {
    metadata_count > 0
}

/// Format a download count for display.
///
/// Two thresholds, both **strict**, so exactly 1,000 prints as `1000` and
/// 1,001 is the first to print as `1K`. The figure is rounded to one decimal
/// by *ceiling*, not by nearest: 1,001 downloads shows as `1.1K`. That reads
/// generously, and it is what the C++ does.
#[must_use]
pub fn format_count(count: i64) -> String {
    if count > 1_000_000 {
        return format!("{}M", ceil_to_tenth(count as f64 / 1_000_000.0));
    }
    if count > 1_000 {
        return format!("{}K", ceil_to_tenth(count as f64 / 1_000.0));
    }
    count.to_string()
}

/// Round up to one decimal place, printed the way Qt prints a float: a
/// trailing `.0` is dropped, so 2.0 comes out as `2`.
///
/// No special case is needed for that — Rust's `f64` `Display` already prints
/// a whole number without its fraction, the same way `QString::arg(float)`
/// does. One was written here and removed once a control could not make it
/// fail.
fn ceil_to_tenth(value: f64) -> String {
    format!("{}", (value * 10.0).ceil() / 10.0)
}
