//! The theme picker: which themes are listed, in which of the two sections,
//! and what each row offers.
//!
//! A port of `theme-list-model.cpp` and `theme-view-host.cpp`
//! (`src/server/src/builtins/theme/`), without the widgets.
//!
//! # Moving the selection changes the theme
//!
//! The view applies a theme as soon as its row is selected and restores the
//! configured one in `beforePop`, so browsing the list is a live preview and
//! leaving it is an undo. [`selection`] and [`leaving`] are that pair; a port
//! that only applied the theme on "enter" would look reasonable and feel
//! completely different.

use compass_search::{Query, WeightedField, score_weighted};

/// The search field's placeholder.
pub const PLACEHOLDER: &str = "Search for a theme...";
/// The first section's heading.
pub const CURRENT_SECTION: &str = "Current Theme";
/// The second section's heading.
pub const AVAILABLE_SECTION: &str = "Available Themes";
/// What a theme with no description of its own shows.
///
/// Verbatim: the C++ subtitle is `tr("Default theme description")`, which is
/// the string itself and not a placeholder for one.
pub const DEFAULT_DESCRIPTION: &str = "Default theme description";

/// The action that applies a theme permanently.
pub const SET_THEME_TITLE: &str = "Set theme";
/// The action that opens the theme file in a text editor.
pub const OPEN_FILE_TITLE: &str = "Open theme file";
/// The action that copies the theme id.
pub const COPY_ID_TITLE: &str = "Copy ID";
/// The action that copies the path to the theme file.
pub const COPY_PATH_TITLE: &str = "Copy path";

/// `Keybind::OpenAction`'s config id.
pub const OPEN_KEYBIND: &str = "action.open";
/// `Keybind::CopyNameAction`'s config id.
pub const COPY_NAME_KEYBIND: &str = "action.copy-name";
/// `Keybind::CopyPathAction`'s config id.
pub const COPY_PATH_KEYBIND: &str = "action.copy-path";

/// The eight palette swatches a row shows, in the C++'s order.
///
/// `customData` maps `PaletteColor0..7` to these semantic colours; the order is
/// what the row draws left to right, and it is not the palette's own order.
pub const PALETTE_ORDER: [&str; 8] = [
    "Red",
    "Blue",
    "Cyan",
    "Green",
    "Magenta",
    "Orange",
    "Foreground",
    "TextMuted",
];

/// A theme as the picker sees it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Theme {
    /// The theme id, which is also what the config stores.
    pub id: String,
    /// Its name.
    pub name: String,
    /// Its description; empty means [`DEFAULT_DESCRIPTION`] is shown.
    pub description: String,
    /// Its icon, when it ships one.
    pub icon: Option<String>,
    /// The file it was read from, when it came from one.
    pub path: Option<String>,
}

impl Theme {
    /// The subtitle the row shows.
    #[must_use]
    pub fn subtitle(&self) -> &str {
        if self.description.is_empty() {
            DEFAULT_DESCRIPTION
        } else {
            &self.description
        }
    }
}

/// The two sections, filled.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ThemeList {
    /// The configured theme, when it survived the filter. At most one.
    pub current: Vec<Theme>,
    /// Everything else, best match first when there is a query.
    pub available: Vec<Theme>,
}

/// Splits and orders `themes` for `query`.
///
/// Three rules, each of which a rewrite could get wrong:
///
/// * the filter runs **before** the split, so the configured theme disappears
///   from the list entirely when it does not match what was typed — the
///   "Current Theme" section is empty rather than pinned;
/// * with an empty query nothing is sorted at all: `stable_sort` is inside
///   `if (!query.empty())`, so the service's own order shows through;
/// * only the name and description are scored, at 1.0 and 0.5. The id is not
///   searchable, so typing a theme's id finds nothing unless it also appears in
///   the name.
#[must_use]
pub fn split(themes: &[Theme], query: &str, current_id: &str) -> ThemeList {
    let parsed = Query::new(query);
    let mut list = ThemeList::default();
    let mut scored: Vec<(u32, Theme)> = Vec::new();

    for theme in themes {
        let mut score = 0;
        if !query.is_empty() {
            let matched = score_weighted(
                &[
                    WeightedField::new(&theme.name, 1.0),
                    WeightedField::new(&theme.description, 0.5),
                ],
                &parsed,
            );
            if !matched.accepted() {
                continue;
            }
            score = matched.score;
        }

        if theme.id == current_id {
            list.current.push(theme.clone());
        } else {
            scored.push((score, theme.clone()));
        }
    }

    if !query.is_empty() {
        scored.sort_by(|a, b| b.0.cmp(&a.0));
    }

    list.available = scored.into_iter().map(|(_, theme)| theme).collect();
    list
}

/// What a row's action panel offers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThemeAction {
    /// The action's title.
    pub title: &'static str,
    /// Its keybind's config id, when it has one.
    pub keybind: Option<&'static str>,
}

/// The panel for one theme.
///
/// "Open theme file" needs both a file and something to open it with; "Copy
/// path" needs only the file. A theme built into the binary has neither.
#[must_use]
pub fn action_panel(theme: &Theme, text_editor: bool) -> Vec<ThemeAction> {
    let mut actions = vec![ThemeAction {
        title: SET_THEME_TITLE,
        keybind: None,
    }];

    if theme.path.is_some() && text_editor {
        actions.push(ThemeAction {
            title: OPEN_FILE_TITLE,
            keybind: Some(OPEN_KEYBIND),
        });
    }

    actions.push(ThemeAction {
        title: COPY_ID_TITLE,
        keybind: Some(COPY_NAME_KEYBIND),
    });

    if theme.path.is_some() {
        actions.push(ThemeAction {
            title: COPY_PATH_TITLE,
            keybind: Some(COPY_PATH_KEYBIND),
        });
    }

    actions
}

/// What selecting a row does: apply that theme, immediately.
///
/// `setOnThemeSelected(... setTheme(theme->id()))` on both sections.
#[must_use]
pub fn selection(theme: &Theme) -> String {
    theme.id.clone()
}

/// What leaving the view does: apply the *configured* theme again.
///
/// `beforePop` reads `config.systemTheme().name`, so every preview is undone —
/// including one the user is still looking at when they press escape.
#[must_use]
pub fn leaving(configured_id: &str) -> String {
    configured_id.to_owned()
}
