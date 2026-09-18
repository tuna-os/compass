//! The window and workspace switchers.
//!
//! Ports `src/server/src/builtins/wm/` — which commands the window-management
//! extension offers, how an open window and a workspace are described in the
//! list, and what each one's action panel holds.
//!
//! None of the window manager itself is here. This is the part that turns a
//! list of windows into rows, which is the part with the decisions in it.

/// A capability the compositor may or may not have.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Capabilities {
    /// Whether the compositor has workspaces at all.
    pub workspaces: bool,
    /// Whether a window can be made fullscreen.
    pub fullscreen: bool,
    /// Whether a window can be floated.
    pub toggle_floating: bool,
    /// Whether an overview can be shown.
    pub toggle_overview: bool,
    /// Whether a window can be pinned across workspaces.
    pub set_sticky: bool,
    /// Whether a window can be moved to another workspace.
    pub move_to_workspace: bool,
}

/// The commands the extension registers, given what the compositor can do.
///
/// Switching windows is unconditional — every compositor the launcher talks to
/// can at least list and focus windows. Everything else is gated, because a
/// command that is certain to fail is worse than a command that is absent.
///
/// `set_sticky` gates nothing here: the C++ tests it and registers no command
/// in the branch. That is left as it is rather than tidied away, because the
/// capability *is* used — the window's action panel offers pinning when it is
/// present — and removing the test would lose the only written record that
/// someone meant to add a command here.
#[must_use]
pub fn registered_commands(caps: Capabilities) -> Vec<&'static str> {
    let mut out = vec!["switch-windows"];
    if caps.workspaces {
        out.push("switch-workspaces");
    }
    if caps.fullscreen {
        out.push("toggle-fullscreen");
    }
    if caps.toggle_floating {
        out.push("toggle-floating");
    }
    if caps.toggle_overview {
        out.push("toggle-overview");
    }
    out
}

/// What the workspace switcher is called.
///
/// Windows calls them desktops and everyone else calls them workspaces, and
/// the C++ picks at compile time. Here the platform is an argument, so both
/// namings are reachable from one build and both can be tested.
#[must_use]
pub fn switch_workspaces_name(is_windows: bool) -> &'static str {
    if is_windows {
        "Switch Desktops"
    } else {
        "Switch Workspaces"
    }
}

/// The extra search terms the workspace switcher answers to.
///
/// All four spellings are listed so that someone typing the word their own
/// desktop uses finds the command, whichever word this one uses.
pub const SWITCH_WORKSPACES_KEYWORDS: &[&str] =
    &["workspaces", "desktops", "virtual desktops", "spaces"];

/// An open window, as the switcher sees it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WindowEntry {
    /// The window's own title.
    pub title: String,
    /// Its `WM_CLASS`, which is how an application is recognised.
    pub wm_class: String,
    /// The matched application's display name, when one was found.
    pub app_name: Option<String>,
    /// The matched application's icon.
    pub app_icon: Option<String>,
    /// The workspace's name, when it has one worth showing.
    pub workspace_name: String,
    /// The workspace's number, when the compositor gave one.
    pub workspace_id: Option<String>,
}

/// The row's title.
#[must_use]
pub fn window_title(entry: &WindowEntry) -> &str {
    &entry.title
}

/// The row's subtitle.
///
/// The application's display name when it was recognised, and otherwise the
/// raw `WM_CLASS` — which is not pretty, but is the only thing that
/// distinguishes two unrecognised windows from each other.
#[must_use]
pub fn window_subtitle(entry: &WindowEntry) -> &str {
    entry.app_name.as_deref().unwrap_or(&entry.wm_class)
}

/// The row's icon.
#[must_use]
pub fn window_icon(entry: &WindowEntry) -> &str {
    entry.app_icon.as_deref().unwrap_or("app-window")
}

/// The row's accessory text.
///
/// A named workspace shows its name; a numbered one shows `WS n`; a window on
/// no workspace shows nothing. The three cases are distinct on purpose — an
/// empty accessory is not the same as `WS ` with nothing after it.
#[must_use]
pub fn window_accessory(entry: &WindowEntry) -> Option<String> {
    if !entry.workspace_name.is_empty() {
        return Some(entry.workspace_name.clone());
    }
    entry.workspace_id.as_ref().map(|id| format!("WS {id}"))
}

/// Whether a workspace's name is worth showing beside a window.
///
/// A compositor that has no names for its workspaces reports the id as the
/// name. Showing that would put `3` where a name belongs and lose the `WS`
/// that says what the number means, so a name equal to the id is treated as
/// no name and the numbered accessory is used instead.
#[must_use]
pub fn workspace_name_worth_showing(name: &str, id: &str) -> bool {
    name != id
}

/// The fuzzy-search fields of a window row, as (text, weight).
///
/// The title carries the most weight because it is what changes between two
/// windows of the same application. The `WM_CLASS` is searched at a low weight
/// even when an application was recognised: someone who knows a window as
/// `org.gnome.Nautilus` should still find it when the desktop entry calls it
/// Files.
#[must_use]
pub fn window_search_fields(entry: &WindowEntry) -> Vec<(&str, f32)> {
    let mut fields = vec![(entry.title.as_str(), 1.0)];
    if let Some(name) = &entry.app_name {
        fields.push((name.as_str(), 0.5));
    }
    fields.push((entry.wm_class.as_str(), 0.3));
    fields
}

/// The window row's action panel, as sections.
#[must_use]
pub fn window_action_panel(entry: &WindowEntry, caps: Capabilities) -> Vec<Vec<&'static str>> {
    let mut window_section = vec!["focus"];
    if caps.set_sticky {
        window_section.push("pin");
    }
    if caps.move_to_workspace {
        window_section.push("bring-to-workspace");
    }
    window_section.push("close");

    let mut panel = vec![window_section];

    // Quitting belongs to the application, not the window, so it is only
    // offered when a window was actually matched to one — there is nothing to
    // quit when all that is known is a `WM_CLASS`.
    if entry.app_name.is_some() {
        panel.push(vec!["quit-app", "force-quit-app"]);
    }

    panel
}

/// The keyboard shortcut that closes a window from the switcher.
pub const CLOSE_WINDOW_SHORTCUT: &str = "ctrl+q";

/// A workspace, as the switcher sees it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WorkspaceEntry {
    /// What it is called.
    pub name: String,
    /// The screen it is on, when one matched.
    pub screen_name: Option<String>,
    /// How many windows are on it.
    pub window_count: usize,
    /// The applications with a window on it, deduplicated, in window order.
    pub app_names: Vec<String>,
}

/// The workspace row's subtitle.
///
/// A count with its plural, or `empty` — not `0 windows`, which reads as a
/// measurement where `empty` reads as a state. The screen is appended when the
/// workspace could be matched to one.
///
/// # A declared divergence
///
/// The C++ writes this as Qt's `tr("%n window(s)", "", n)`. Qt only applies
/// plural forms when a translation supplies them, and there is no English
/// entry for this string — Russian and Ukrainian have real plural forms, and
/// English falls back to the source text. So the shipped English reads
/// `3 window(s)`, with the translator's placeholder left in the interface.
///
/// This produces `1 window` and `3 windows` instead: what every translated
/// locale already does, and what English would do if the entry existed. The
/// difference is pinned by a test rather than left to be rediscovered.
#[must_use]
pub fn workspace_subtitle(entry: &WorkspaceEntry) -> String {
    let mut text = match entry.window_count {
        0 => "empty".to_owned(),
        1 => "1 window".to_owned(),
        n => format!("{n} windows"),
    };
    if let Some(screen) = &entry.screen_name {
        text.push_str(" - ");
        text.push_str(screen);
    }
    text
}

/// The applications shown as accessories on a workspace row.
///
/// One per distinct application, in the order their windows were listed. The
/// count is the number of *windows*, so a workspace with three terminals shows
/// one icon and says three.
#[must_use]
pub fn workspace_accessories(entry: &WorkspaceEntry) -> &[String] {
    &entry.app_names
}

/// Deduplicate the applications on a workspace, keeping first appearance.
///
/// Windows with no matched application contribute nothing: an unrecognised
/// window is counted but has no icon to show.
#[must_use]
pub fn workspace_apps(window_apps: &[Option<String>]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for app in window_apps.iter().flatten() {
        if !out.contains(app) {
            out.push(app.clone());
        }
    }
    out
}

/// The fuzzy-search fields of a workspace row, as (text, weight).
///
/// The applications on a workspace are searchable at a low weight, which is
/// what lets someone find "the workspace with the browser on it" without
/// knowing what it is called.
#[must_use]
pub fn workspace_search_fields(entry: &WorkspaceEntry) -> Vec<(&str, f32)> {
    let mut fields = vec![(entry.name.as_str(), 1.0)];
    if let Some(screen) = &entry.screen_name {
        fields.push((screen.as_str(), 0.8));
    }
    for app in &entry.app_names {
        fields.push((app.as_str(), 0.3));
    }
    fields
}

/// What the workspace row's only action is called.
#[must_use]
pub fn switch_to_workspace_label(is_windows: bool) -> &'static str {
    if is_windows {
        "Switch to desktop"
    } else {
        "Switch to workspace"
    }
}

/// The search box's placeholder in the workspace switcher.
#[must_use]
pub fn workspace_search_placeholder(is_windows: bool) -> &'static str {
    if is_windows {
        "Search desktops..."
    } else {
        "Search workspaces..."
    }
}
