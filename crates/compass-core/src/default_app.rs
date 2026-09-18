//! The two "pick a default" views: which applications are offered, in what
//! order, and what happens when one is chosen.
//!
//! A port of `set-default-browser-view-host.hpp` and
//! `set-default-terminal-view-host.hpp`
//! (`src/server/src/builtins/system/`), without the widgets.
//!
//! The two are near-copies of each other and differ in three ways that matter,
//! all of them pinned below: how the candidates are found, what a successful
//! change says, and — in the C++ — whether the sort is stable.

/// The URL the browser picker asks "what opens this?".
///
/// The list is not "applications that look like browsers" but "applications
/// registered for an `https` URL", which is why an entry that handles links
/// without being a browser shows up here.
pub const BROWSER_PROBE_URL: &str = "https://vicinae.com";

/// The browser picker's search placeholder.
pub const BROWSER_PLACEHOLDER: &str = "Select a web browser...";
/// The browser picker's section heading.
pub const BROWSER_SECTION: &str = "Available web browsers";
/// The browser picker's action.
pub const BROWSER_ACTION: &str = "Set as default browser";
/// What the HUD says on success.
pub const BROWSER_SUCCESS: &str = "Default browser changed";
/// What the toast says on failure.
pub const BROWSER_FAILURE: &str = "Failed to set default browser";

/// The terminal picker's search placeholder.
pub const TERMINAL_PLACEHOLDER: &str = "Select a terminal emulator...";
/// The terminal picker's section heading.
pub const TERMINAL_SECTION: &str = "Available terminal emulators";
/// The terminal picker's action.
pub const TERMINAL_ACTION: &str = "Set as default terminal";
/// What the HUD says on success.
pub const TERMINAL_SUCCESS: &str = "Default terminal changed";
/// What the toast says on failure.
pub const TERMINAL_FAILURE: &str = "Failed to set default terminal";

/// One candidate application.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PickerApp {
    /// The desktop-entry id.
    pub id: String,
    /// What to show.
    pub display_name: String,
    /// The second line.
    pub description: String,
    /// `displayable()`; a `NoDisplay` entry is not offered.
    pub displayable: bool,
    /// `isTerminalEmulator()`; only the terminal picker reads it.
    pub terminal_emulator: bool,
}

/// A row in a picker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PickerItem {
    /// The application.
    pub app: PickerApp,
    /// Whether it is the current default; the view draws a green check.
    pub is_default: bool,
}

/// Everything a picker view needs that is not a widget.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Picker {
    /// The search field's placeholder.
    pub placeholder: &'static str,
    /// The section heading.
    pub section: &'static str,
    /// The action on every row.
    pub action: &'static str,
    /// What the HUD says when the change lands.
    pub success: &'static str,
    /// What the toast says when it does not.
    pub failure: &'static str,
    /// The rows: the current default first, then the rest.
    pub items: Vec<PickerItem>,
}

/// The browser picker.
///
/// `openers` is what the application database says can open
/// [`BROWSER_PROBE_URL`], in its own order.
#[must_use]
pub fn browser_picker(openers: &[PickerApp], default_id: Option<&str>) -> Picker {
    Picker {
        placeholder: BROWSER_PLACEHOLDER,
        section: BROWSER_SECTION,
        action: BROWSER_ACTION,
        success: BROWSER_SUCCESS,
        failure: BROWSER_FAILURE,
        items: default_first(
            openers.iter().filter(|app| app.displayable).cloned(),
            default_id,
        ),
    }
}

/// The terminal picker.
///
/// `apps` is the whole application list; the filter is `displayable() &&
/// isTerminalEmulator()`, so this one does not ask the MIME database anything.
#[must_use]
pub fn terminal_picker(apps: &[PickerApp], default_id: Option<&str>) -> Picker {
    Picker {
        placeholder: TERMINAL_PLACEHOLDER,
        section: TERMINAL_SECTION,
        action: TERMINAL_ACTION,
        success: TERMINAL_SUCCESS,
        failure: TERMINAL_FAILURE,
        items: default_first(
            apps.iter()
                .filter(|app| app.displayable && app.terminal_emulator)
                .cloned(),
            default_id,
        ),
    }
}

/// The current default first, everything else in the order it arrived.
///
/// The comparator is `isDefault(a) > isDefault(b)` and nothing else, so it says
/// nothing about the order of the rest. The browser view sorts *stably*, which
/// keeps that order; the terminal view uses `std::ranges::sort`, which does not,
/// so its list below the default is unspecified — two runs of the same C++
/// binary may disagree. Both are stable here. See PARITY.md.
fn default_first(
    apps: impl Iterator<Item = PickerApp>,
    default_id: Option<&str>,
) -> Vec<PickerItem> {
    let mut items: Vec<PickerItem> = apps
        .map(|app| {
            let is_default = default_id.is_some_and(|id| id == app.id);
            PickerItem { app, is_default }
        })
        .collect();

    items.sort_by_key(|item| !item.is_default);
    items
}
