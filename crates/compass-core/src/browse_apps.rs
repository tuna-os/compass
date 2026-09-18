//! The "Search Applications" builtin's model: how apps are scored, what the
//! row shows, and what the action panel offers.
//!
//! A port of `BrowseAppsSection`
//! (`src/server/src/builtins/system/browse-apps-model.{hpp,cpp}`) without the
//! view. The C++ file is half Qt widgets and half decisions; this is the
//! decisions, as data a renderer can lay out.
//!
//! # Not the same weights as the root list
//!
//! This builtin scores keywords at **0.3**, where
//! [`crate::root_items`] scores them at 0.6 — the root list is competing
//! against every other kind of item, and this list is only competing with other
//! applications. Both are pinned, because a single shared constant would be a
//! change of behaviour dressed up as a tidy-up.

use compass_search::{Match, Query, WeightedField, score_weighted};

/// The section heading, with `{count}` where the number goes.
///
/// Kept as the C++ string rather than formatted here, because that is what a
/// translator sees: `tr("Applications ({count})")`.
pub const SECTION_TITLE: &str = "Applications ({count})";

/// The accessory shown on an application the desktop file hides.
pub const HIDDEN_ACCESSORY: &str = "Hidden";

/// The title of the action that launches the application.
pub const OPEN_TITLE: &str = "Open Application";

/// The title of the action that copies the desktop-entry id.
pub const COPY_ID_TITLE: &str = "Copy App ID";

/// The title of the action that copies the path to the desktop file.
pub const COPY_LOCATION_TITLE: &str = "Copy App Location";

/// How many desktop actions get a numbered shortcut: `i < 9`.
pub const NUMBERED_ACTION_LIMIT: usize = 9;

/// One of the application's own desktop actions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesktopAction {
    /// The action's id within the desktop entry.
    pub id: String,
    /// What to call it.
    pub display_name: String,
}

/// What this builtin needs to know about an application.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BrowseApp {
    /// The desktop-entry id.
    pub id: String,
    /// `displayName()`; weight 1.0.
    pub display_name: String,
    /// `description()`; weight 0.5.
    pub description: String,
    /// `keywords()`; weight 0.3 each.
    pub keywords: Vec<String>,
    /// The path to the desktop file.
    pub path: String,
    /// `displayable()`; false for `NoDisplay=true` entries.
    pub displayable: bool,
    /// Its desktop actions, in entry order.
    pub actions: Vec<DesktopAction>,
}

/// Scores `app` against `query`, with this builtin's own field weights.
#[must_use]
pub fn score(app: &BrowseApp, query: &Query) -> Match {
    let mut fields = vec![
        WeightedField::new(&app.display_name, 1.0),
        WeightedField::new(&app.description, 0.5),
    ];
    fields.extend(
        app.keywords
            .iter()
            .map(|keyword| WeightedField::new(keyword, 0.3)),
    );

    score_weighted(&fields, query)
}

/// The accessories on the row: one, or none.
#[must_use]
pub fn accessories(app: &BrowseApp) -> Vec<&'static str> {
    if app.displayable {
        Vec::new()
    } else {
        vec![HIDDEN_ACCESSORY]
    }
}

/// Which half of the panel an action sits in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    /// `panel->createSection()`, the first one: what you came here to do.
    Main,
    /// The second: the things you occasionally want.
    Utils,
}

/// A keyboard shortcut on a panel action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Shortcut {
    /// A literal chord, as `QString("control+shift+%1")` builds.
    Literal(String),
    /// One of the launcher's named keybinds, by its config id — the user can
    /// rebind it, so the literal chord is not the contract.
    Keybind(&'static str),
}

/// `Keybind::OpenAction`'s config id, and its default chord.
pub const OPEN_KEYBIND: &str = "action.open";
/// What `action.open` is bound to out of the box: `Qt::Key_O | ControlModifier`.
pub const OPEN_KEYBIND_DEFAULT: &str = "control+o";

/// What an action does when it fires.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActionKind {
    /// Raise a window the application already has open.
    FocusWindow {
        /// The window manager's id for it.
        window: String,
    },
    /// Launch the application.
    OpenApp,
    /// Launch one of its desktop actions.
    OpenDesktopAction {
        /// The action's id within the desktop entry.
        id: String,
    },
    /// Open the directory holding the desktop file.
    OpenLocation,
    /// Copy the desktop-entry id.
    CopyAppId,
    /// Copy the path to the desktop file.
    CopyAppLocation,
}

/// One entry in the action panel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PanelAction {
    /// What it does.
    pub kind: ActionKind,
    /// What it is called.
    pub title: String,
    /// Its shortcut, if it has one.
    pub shortcut: Option<Shortcut>,
    /// Which section it belongs to.
    pub section: Section,
    /// Whether firing it empties the search bar. Only the open action sets it.
    pub clear_search: bool,
}

/// The whole panel: a title and the actions, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionPanel {
    /// `panel->setTitle(app->displayName())`.
    pub title: String,
    /// Main section first, then utils, each in the order it was built.
    pub actions: Vec<PanelAction>,
}

/// Builds the action panel, as `BrowseAppsSection::buildActionPanel` does.
///
/// `windows` are the application's open windows, in the window manager's order;
/// `location_opener` says whether anything can open the directory the desktop
/// file lives in, which is what gates the "open location" action.
#[must_use]
pub fn action_panel(app: &BrowseApp, windows: &[String], location_opener: bool) -> ActionPanel {
    let mut actions = Vec::new();

    // Only the first window, and only when there is one: the C++ takes
    // `activeWindows.front()`.
    if let Some(window) = windows.first() {
        actions.push(PanelAction {
            kind: ActionKind::FocusWindow {
                window: window.clone(),
            },
            title: "Focus Window".to_owned(),
            shortcut: None,
            section: Section::Main,
            clear_search: false,
        });
    }

    actions.push(PanelAction {
        kind: ActionKind::OpenApp,
        title: OPEN_TITLE.to_owned(),
        shortcut: None,
        section: Section::Main,
        // `open->setClearSearch(true)` -- launching something is the end of
        // the search, and coming back to a stale query would be wrong.
        clear_search: true,
    });

    for (index, action) in app.actions.iter().enumerate() {
        actions.push(PanelAction {
            kind: ActionKind::OpenDesktopAction {
                id: action.id.clone(),
            },
            title: action.display_name.clone(),
            // `if (i < 9) action->setShortcut(QString("control+shift+%1").arg(i + 1));`
            // -- one-based, and the tenth action onward gets nothing.
            shortcut: (index < NUMBERED_ACTION_LIMIT)
                .then(|| Shortcut::Literal(format!("control+shift+{}", index + 1))),
            section: Section::Main,
            clear_search: false,
        });
    }

    if location_opener {
        actions.push(PanelAction {
            kind: ActionKind::OpenLocation,
            title: "Open Location".to_owned(),
            shortcut: Some(Shortcut::Keybind(OPEN_KEYBIND)),
            section: Section::Utils,
            clear_search: false,
        });
    }

    actions.push(PanelAction {
        kind: ActionKind::CopyAppId,
        title: COPY_ID_TITLE.to_owned(),
        shortcut: None,
        section: Section::Utils,
        clear_search: false,
    });
    actions.push(PanelAction {
        kind: ActionKind::CopyAppLocation,
        title: COPY_LOCATION_TITLE.to_owned(),
        shortcut: None,
        section: Section::Utils,
        clear_search: false,
    });

    ActionPanel {
        title: app.display_name.clone(),
        actions,
    }
}
