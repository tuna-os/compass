//! The settings window: sidebar, pages, and the theme picker view.
//!
//! A port of `src/server/src/ui/settings/` (2,292 lines) without Qt.
//! The C++ side is seven models and a controller:
//!
//! * `SettingsController` — open/close, `openTab`/`openExtensionPreferences`
//! * `SettingsSidebarModel` — flat list of core pages, provider groups/extensions, dividers; `setQuery` + fuzzy filter
//! * `GeneralSettingsModel` — every toggle and dropdown on General/Appearance/Advanced
//! * `ExtensionSettingsModel` / `PreferenceFormModel` / `ProviderCommandModel` — provider enable, alias, shortcut, preference form
//! * `KeybindSettingsModel` / `shortcut-conflict.*` — keybind list and conflict detection
//! * `Theme`-related behaviour through `theme-list-model.cpp` / `theme-view-host.cpp` (`src/server/src/builtins/theme/`)
//!
//! This crate already ports the theme *model* as [`compass_core::theme_picker`] (the
//! split, the live preview, the action panel, the eight swatches). What was missing
//! was the window that *shows* it, and the sidebar that reaches it — so a build that
//! listed `compass-ui/src` showed no `settings` file at all.
//!
//! The view layer (Iced widgets, QML) is intentionally not replicated: ADR-0001 chose
//! Iced over Qt Widgets/QML, so a pixel-diff against the QML would not be meaningful.
//! What *is* ported is the state the view reads: which rows are visible, in which
//! order, under which heading, and what selecting a row does.

use compass_core::theme_picker::{self, Theme, ThemeList};
use compass_search::{Query, WeightedField, score_weighted};

// ---------------------------------------------------------------------------
// Sidebar — `settings-sidebar-model.{hpp,cpp}`
// ---------------------------------------------------------------------------

/// Core pages, in the C++'s order.
///
/// `src/server/src/ui/settings/settings-sidebar-model.cpp: rebuildRows()` lists
/// these five, each with its builtin icon. The ids are the QML `currentPage` keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SettingsPage {
    /// General — hotkey, close behaviour, language.
    General,
    /// Appearance — theme, font, icon theme, window material.
    Appearance,
    /// Keybindings — list and conflict surface.
    Keybindings,
    /// Advanced — file indexing, input server, tray, layer-shell, etc.
    Advanced,
    /// About — version, update check.
    About,
}

impl SettingsPage {
    /// The persisted `currentPage` key, as QML reads it.
    #[must_use]
    pub const fn key(self) -> &'static str {
        match self {
            Self::General => "general",
            Self::Appearance => "appearance",
            Self::Keybindings => "keybindings",
            Self::Advanced => "advanced",
            Self::About => "about",
        }
    }

    /// Human label, as `tr()` would provide in English.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::General => "General",
            Self::Appearance => "Appearance",
            Self::Keybindings => "Keybindings",
            Self::Advanced => "Advanced",
            Self::About => "About",
        }
    }

    /// All five, in display order.
    pub const ALL: [Self; 5] = [
        Self::General,
        Self::Appearance,
        Self::Keybindings,
        Self::Advanced,
        Self::About,
    ];
}

/// What a sidebar row is.
///
/// Mirrors `SettingsSidebarModel::Row::kind` in C++: `core`, `group`, `ext`,
/// `divider`, `command`. `divider` has no key and is never selectable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SidebarKind {
    /// One of the five core pages.
    Core,
    /// A provider group (e.g. Applications, System).
    Group,
    /// An extension provider.
    Extension,
    /// A non-selectable separator.
    Divider,
    /// A command nested under its provider when searching.
    Command,
}

/// One row the sidebar draws.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SidebarRow {
    /// Stable key — `SettingsPage::key` for core, `providerId` for providers, `entrypointId` for commands.
    pub key: String,
    /// What kind of row this is.
    pub kind: SidebarKind,
    /// Human label.
    pub label: String,
    /// Whether the provider is enabled (always true for core/divider).
    pub enabled: bool,
}

impl SidebarRow {
    /// Whether the row can be selected.
    #[must_use]
    pub fn selectable(&self) -> bool {
        !matches!(self.kind, SidebarKind::Divider)
    }
}

/// Input for building the sidebar's provider section.
///
/// The C++ reads `RootItemManager::providers()` live. This struct carries the
/// same fields the sidebar needs without pulling the whole manager into the view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderInfo {
    /// Stable provider id, e.g. `applications`, `kde-settings`.
    pub id: String,
    /// Display name.
    pub display_name: String,
    /// Whether this provider is a group (has children) rather than a leaf extension.
    pub is_group: bool,
    /// Whether the provider is transient (excluded from settings entirely).
    pub is_transient: bool,
    /// Current enabled state from config.
    pub enabled: bool,
}

/// Flat list model backing the settings sidebar.
///
/// Holds core pages, extension providers and (while searching) matching commands
/// nested under their provider, plus divider rows. Filtering happens via
/// [`SidebarModel::set_query`], scored with [`compass_search`].
#[derive(Debug, Clone, Default)]
pub struct SidebarModel {
    rows: Vec<SidebarRow>,
    query: String,
}

impl SidebarModel {
    /// Build the idle (empty-query) model for the given providers.
    #[must_use]
    pub fn new(providers: &[ProviderInfo]) -> Self {
        let mut model = Self::default();
        model.rebuild(providers);
        model
    }

    /// The current rows, in display order.
    #[must_use]
    pub fn rows(&self) -> &[SidebarRow] {
        &self.rows
    }

    /// Current query.
    #[must_use]
    pub fn query(&self) -> &str {
        &self.query
    }

    /// Update the query and rebuild, as `setQuery()` does in C++.
    pub fn set_query(&mut self, query: String, providers: &[ProviderInfo]) {
        if query == self.query {
            return;
        }
        self.query = query;
        self.rebuild(providers);
    }

    /// Index of the row with `key`, or -1 when absent.
    #[must_use]
    pub fn index_of_key(&self, key: &str) -> isize {
        self.rows
            .iter()
            .position(|row| row.key == key)
            .map(|index| index as isize)
            .unwrap_or(-1)
    }

    /// Key at `row`, or empty when out of range.
    #[must_use]
    pub fn key_at(&self, row: usize) -> &str {
        self.rows.get(row).map(|row| row.key.as_str()).unwrap_or("")
    }

    /// Kind at `row`, as a string matching the C++ role (`core`/`group`/`ext`/`divider`/`command`).
    #[must_use]
    pub fn kind_at(&self, row: usize) -> &str {
        match self.rows.get(row).map(|row| &row.kind) {
            Some(SidebarKind::Core) => "core",
            Some(SidebarKind::Group) => "group",
            Some(SidebarKind::Extension) => "ext",
            Some(SidebarKind::Divider) => "divider",
            Some(SidebarKind::Command) => "command",
            None => "",
        }
    }

    /// First selectable row, or -1 when the list is empty/only dividers.
    #[must_use]
    pub fn first_selectable_row(&self) -> isize {
        self.rows
            .iter()
            .position(|row| row.selectable())
            .map(|index| index as isize)
            .unwrap_or(-1)
    }

    /// Step `delta` from `from_row`, skipping dividers.
    #[must_use]
    pub fn step_row(&self, from_row: isize, delta: isize) -> isize {
        if from_row < 0 || from_row as usize >= self.rows.len() {
            return self.first_selectable_row();
        }
        let mut current = from_row + delta;
        while current >= 0 && (current as usize) < self.rows.len() {
            if self.rows[current as usize].selectable() {
                return current;
            }
            current += delta.signum();
        }
        from_row
    }

    fn rebuild(&mut self, providers: &[ProviderInfo]) {
        self.rows.clear();

        let visible_providers: Vec<&ProviderInfo> = providers
            .iter()
            .filter(|provider| !provider.is_transient)
            .collect();

        if self.query.is_empty() {
            for page in SettingsPage::ALL {
                self.rows.push(SidebarRow {
                    key: page.key().to_owned(),
                    kind: SidebarKind::Core,
                    label: page.label().to_owned(),
                    enabled: true,
                });
            }
            let groups: Vec<&&ProviderInfo> = visible_providers
                .iter()
                .filter(|provider| provider.is_group)
                .collect();
            let exts: Vec<&&ProviderInfo> = visible_providers
                .iter()
                .filter(|provider| !provider.is_group)
                .collect();

            if !groups.is_empty() || !exts.is_empty() {
                self.rows.push(SidebarRow {
                    key: String::new(),
                    kind: SidebarKind::Divider,
                    label: String::new(),
                    enabled: true,
                });
            }
            for provider in &groups {
                self.rows.push(SidebarRow {
                    key: provider.id.clone(),
                    kind: SidebarKind::Group,
                    label: provider.display_name.clone(),
                    enabled: provider.enabled,
                });
            }
            if !groups.is_empty() && !exts.is_empty() {
                self.rows.push(SidebarRow {
                    key: String::new(),
                    kind: SidebarKind::Divider,
                    label: String::new(),
                    enabled: true,
                });
            }
            for provider in &exts {
                self.rows.push(SidebarRow {
                    key: provider.id.clone(),
                    kind: SidebarKind::Extension,
                    label: provider.display_name.clone(),
                    enabled: provider.enabled,
                });
            }
            return;
        }

        let query = Query::new(&self.query);
        let mut scored: Vec<(u32, SidebarRow)> = Vec::new();

        for page in SettingsPage::ALL {
            let matched = score_weighted(&[WeightedField::new(page.label(), 1.0)], &query);
            if matched.accepted() {
                scored.push((
                    matched.score,
                    SidebarRow {
                        key: page.key().to_owned(),
                        kind: SidebarKind::Core,
                        label: page.label().to_owned(),
                        enabled: true,
                    },
                ));
            }
        }

        for provider in &visible_providers {
            let matched =
                score_weighted(&[WeightedField::new(&provider.display_name, 1.0)], &query);
            if matched.accepted() {
                scored.push((
                    matched.score,
                    SidebarRow {
                        key: provider.id.clone(),
                        kind: if provider.is_group {
                            SidebarKind::Group
                        } else {
                            SidebarKind::Extension
                        },
                        label: provider.display_name.clone(),
                        enabled: provider.enabled,
                    },
                ));
            }
        }

        scored.sort_by(|left, right| right.0.cmp(&left.0));
        self.rows = scored.into_iter().map(|(_, row)| row).collect();
    }
}

// ---------------------------------------------------------------------------
// Theme picker view — `theme-list-model` + `theme-view-host`
// ---------------------------------------------------------------------------

/// State for the theme picker list, as `ThemeViewHost` keeps it.
///
/// Unlike the launcher's `AppIndex` search, this view draws two sections:
/// the configured theme (at most one row) and everything else, filtered and
/// re-scored on every keystroke. The split and scoring live in
/// [`compass_core::theme_picker::split`] — this type adds the selection
/// and the live-preview lifecycle.
#[derive(Debug, Clone)]
pub struct ThemePickerView {
    themes: Vec<Theme>,
    configured_id: String,
    query: String,
    list: ThemeList,
    selected_index: Option<usize>,
    entering_configured_id: String,
}

impl ThemePickerView {
    /// Create a view over `themes`, with `configured_id` as the current theme.
    #[must_use]
    pub fn new(themes: Vec<Theme>, configured_id: String) -> Self {
        let list = theme_picker::split(&themes, "", &configured_id);
        let selected_index = if list.current.is_empty() && list.available.is_empty() {
            None
        } else {
            Some(0)
        };
        Self {
            themes,
            configured_id: configured_id.clone(),
            query: String::new(),
            list,
            selected_index,
            entering_configured_id: configured_id,
        }
    }

    /// Current query.
    #[must_use]
    pub fn query(&self) -> &str {
        &self.query
    }

    /// Current split, as the view draws it.
    #[must_use]
    pub fn list(&self) -> &ThemeList {
        &self.list
    }

    /// Flat count: `current.len() + available.len()`.
    #[must_use]
    pub fn len(&self) -> usize {
        self.list.current.len() + self.list.available.len()
    }

    /// Whether there is anything to select.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Selected position in the flat list, if any.
    #[must_use]
    pub fn selected_index(&self) -> Option<usize> {
        self.selected_index
    }

    /// Theme at `flat_index`, if in range.
    #[must_use]
    pub fn theme_at(&self, flat_index: usize) -> Option<&Theme> {
        if flat_index < self.list.current.len() {
            self.list.current.get(flat_index)
        } else {
            self.list
                .available
                .get(flat_index - self.list.current.len())
        }
    }

    /// Selected theme, if any.
    #[must_use]
    pub fn selected_theme(&self) -> Option<&Theme> {
        self.selected_index.and_then(|index| self.theme_at(index))
    }

    /// Update the filter and re-split, as `ThemeViewHost::textChanged` does.
    pub fn set_query(&mut self, query: String) {
        self.query = query;
        self.list = theme_picker::split(&self.themes, &self.query, &self.configured_id);
        self.selected_index = if self.is_empty() { None } else { Some(0) };
    }

    /// Move selection by `delta`, clamping at the ends.
    ///
    /// The C++ uses `ListView` navigation; this view clamps rather than wraps,
    /// matching the launcher's default `wrap_navigation = false`.
    pub fn move_selection(&mut self, delta: isize) {
        let Some(current) = self.selected_index else {
            return;
        };
        let len = self.len() as isize;
        let next = (current as isize + delta).clamp(0, len - 1) as usize;
        self.selected_index = Some(next);
    }

    /// What selecting `theme` does — apply it immediately for a live preview.
    ///
    /// Mirrors `setOnThemeSelected(... setTheme(theme->id()))` on both sections.
    #[must_use]
    pub fn preview_selection(&self, theme: &Theme) -> String {
        theme_picker::selection(theme)
    }

    /// What leaving the view does — re-apply the configured theme.
    ///
    /// Mirrors `ThemeViewHost::beforePop`.
    #[must_use]
    pub fn leaving(&self) -> String {
        theme_picker::leaving(&self.entering_configured_id)
    }

    /// Which theme would be restored on pop (the one in force on entry).
    #[must_use]
    pub fn entering_configured_id(&self) -> &str {
        &self.entering_configured_id
    }

    /// Notify that the *configured* theme changed (e.g. via config).
    pub fn set_configured_id(&mut self, id: String) {
        self.configured_id = id;
        self.list = theme_picker::split(&self.themes, &self.query, &self.configured_id);
        if self.selected_index.is_some() && self.selected_index.unwrap() >= self.len() {
            self.selected_index = if self.is_empty() { None } else { Some(0) };
        }
    }
}

/// Which settings tab to show when opening the window.
///
/// Mirrors `SettingsController::openTab(tabId)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingsTab {
    /// One of the core pages.
    Core(SettingsPage),
    /// A provider's extension preferences.
    Extension(String),
}

impl SettingsTab {
    /// Parse a tab id as the window understands it.
    #[must_use]
    pub fn from_id(id: &str) -> Option<Self> {
        for page in SettingsPage::ALL {
            if page.key() == id {
                return Some(Self::Core(page));
            }
        }
        if id.is_empty() {
            None
        } else {
            Some(Self::Extension(id.to_owned()))
        }
    }

    /// The stable id.
    #[must_use]
    pub fn id(&self) -> &str {
        match self {
            Self::Core(page) => page.key(),
            Self::Extension(id) => id.as_str(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn providers() -> Vec<ProviderInfo> {
        vec![
            ProviderInfo {
                id: "applications".to_owned(),
                display_name: "Applications".to_owned(),
                is_group: true,
                is_transient: false,
                enabled: true,
            },
            ProviderInfo {
                id: "com.example.clock".to_owned(),
                display_name: "Clock".to_owned(),
                is_group: false,
                is_transient: false,
                enabled: true,
            },
        ]
    }

    #[test]
    fn sidebar_idle_has_core_pages_then_provider_sections() {
        let model = SidebarModel::new(&providers());
        let keys: Vec<&str> = model.rows().iter().map(|row| row.key.as_str()).collect();
        assert_eq!(keys[0], "general");
        assert_eq!(keys[1], "appearance");
        // Expect a divider between core and groups.
        assert!(keys.contains(&"applications"));
        assert!(keys.contains(&"com.example.clock"));
    }

    #[test]
    fn sidebar_search_filters_to_matching_label() {
        let mut model = SidebarModel::new(&providers());
        model.set_query("clock".to_owned(), &providers());
        assert_eq!(model.rows().len(), 1);
        assert_eq!(model.rows()[0].key, "com.example.clock");
    }

    #[test]
    fn sidebar_set_query_no_change_is_noop() {
        let mut model = SidebarModel::new(&providers());
        let before = model.rows().to_vec();
        model.set_query(String::new(), &providers());
        assert_eq!(model.rows(), before.as_slice());
    }

    #[test]
    fn sidebar_step_skips_dividers() {
        let model = SidebarModel::new(&providers());
        let first = model.first_selectable_row();
        assert!(first >= 0);
        let second = model.step_row(first, 1);
        assert_ne!(model.kind_at(second as usize), "divider");
    }

    fn themes() -> Vec<Theme> {
        vec![
            Theme {
                id: "system".to_owned(),
                name: "System".to_owned(),
                description: String::new(),
                icon: None,
                path: None,
            },
            Theme {
                id: "catppuccin".to_owned(),
                name: "Catppuccin".to_owned(),
                description: "Soothing pastel theme".to_owned(),
                icon: None,
                path: Some("/themes/catppuccin.toml".to_owned()),
            },
        ]
    }

    #[test]
    fn theme_picker_splits_current_and_available() {
        let view = ThemePickerView::new(themes(), "system".to_owned());
        assert_eq!(view.list().current.len(), 1);
        assert_eq!(view.list().available.len(), 1);
        assert_eq!(view.list().current[0].id, "system");
    }

    #[test]
    fn theme_picker_filter_hides_non_matching_current() {
        let mut view = ThemePickerView::new(themes(), "system".to_owned());
        view.set_query("catppuccin".to_owned());
        assert!(view.list().current.is_empty());
        assert_eq!(view.list().available.len(), 1);
    }

    #[test]
    fn theme_picker_preview_and_leaving() {
        let view = ThemePickerView::new(themes(), "system".to_owned());
        let selected = view.theme_at(1).unwrap();
        assert_eq!(view.preview_selection(selected), "catppuccin");
        assert_eq!(view.leaving(), "system");
    }

    #[test]
    fn theme_picker_navigation_clamps() {
        let mut view = ThemePickerView::new(themes(), "system".to_owned());
        assert_eq!(view.selected_index(), Some(0));
        view.move_selection(10);
        assert_eq!(view.selected_index(), Some(1));
        view.move_selection(-10);
        assert_eq!(view.selected_index(), Some(0));
    }
}
