//! View state management for the launcher UI.
//!
//! Encapsulates theme, appearance, typography, and view memory state that persists
//! across application sessions and is independent of search, dialog, or window lifecycle.

use crate::design::Appearance;
use std::collections::HashSet;
use std::path::PathBuf;

/// Theme and appearance state that persists across sessions.
///
/// This struct holds all UI presentation state including theme choice,
/// appearance settings, typography configuration, and view memory (window
/// size, position, and other remembered state).
///
/// By extracting this from LauncherApp, we make it independently testable
/// and easier to reason about UI state changes separately from search
/// coordination or dialog management.
pub struct ViewState {
    /// Curated theme (#153).
    theme_choice: crate::theme::Theme,

    /// Where Set Theme looks for theme files.
    theme_dirs: Vec<PathBuf>,

    /// What views remember between openings.
    view_memory: crate::view_memory::ViewMemory,

    /// Previously persisted theme for live-preview cancellation (#153).
    theme_preview: Option<crate::theme::Theme>,

    /// Which palette to draw with. See [`LauncherApp::theme`].
    appearance: Appearance,

    /// Where later appearance changes arrive. See [`AppFlags::appearance_link`].
    appearance_link: Option<crate::appearance::AppearanceLink>,

    /// The desktop's interface font family.
    ///
    /// `None` is the historic hard-coded `Cantarell` path. `Some` is whatever
    /// `org.gnome.desktop.interface font-name` reported, parsed to a family.
    /// See `crate::typography`.
    font_family: Option<String>,

    /// Where later font changes arrive. See [`AppFlags::typography_link`].
    typography_link: Option<crate::typography::TypographyLink>,

    /// Extension icon files seen to exist, so a draw does not stat them.
    known_files: HashSet<PathBuf>,

    /// Masked images, drawn once.
    masked: crate::icons::MaskedCache,
}

impl ViewState {
    /// Create a new ViewState with default values.
    pub fn new(
        theme_choice: crate::theme::Theme,
        appearance: Appearance,
        masked: crate::icons::MaskedCache,
    ) -> Self {
        Self {
            theme_choice,
            theme_dirs: Vec::new(),
            view_memory: crate::view_memory::ViewMemory::default(),
            theme_preview: None,
            appearance,
            appearance_link: None,
            font_family: None,
            typography_link: None,
            known_files: HashSet::new(),
            masked,
        }
    }

    /// Get the current theme choice.
    pub fn theme_choice(&self) -> &crate::theme::Theme {
        &self.theme_choice
    }

    /// Set the theme choice.
    pub fn set_theme_choice(&mut self, theme: crate::theme::Theme) {
        self.theme_choice = theme;
    }

    /// Get the theme directories.
    pub fn theme_dirs(&self) -> &[PathBuf] {
        &self.theme_dirs
    }

    /// Set the theme directories.
    pub fn set_theme_dirs(&mut self, dirs: Vec<PathBuf>) {
        self.theme_dirs = dirs;
    }

    /// Get the view memory.
    pub fn view_memory(&self) -> &crate::view_memory::ViewMemory {
        &self.view_memory
    }

    /// Get mutable access to view memory.
    pub fn view_memory_mut(&mut self) -> &mut crate::view_memory::ViewMemory {
        &mut self.view_memory
    }

    /// Get the theme preview (for cancellation).
    pub fn theme_preview(&self) -> Option<&crate::theme::Theme> {
        self.theme_preview.as_ref()
    }

    /// Set the theme preview.
    pub fn set_theme_preview(&mut self, theme: Option<crate::theme::Theme>) {
        self.theme_preview = theme;
    }

    /// Get the current appearance palette.
    pub fn appearance(&self) -> &Appearance {
        &self.appearance
    }

    /// Set the appearance palette.
    pub fn set_appearance(&mut self, appearance: Appearance) {
        self.appearance = appearance;
    }

    /// Get the appearance link.
    pub fn appearance_link(&self) -> Option<&crate::appearance::AppearanceLink> {
        self.appearance_link.as_ref()
    }

    /// Set the appearance link.
    pub fn set_appearance_link(&mut self, link: Option<crate::appearance::AppearanceLink>) {
        self.appearance_link = link;
    }

    /// Get the font family.
    pub fn font_family(&self) -> Option<&str> {
        self.font_family.as_deref()
    }

    /// Set the font family.
    pub fn set_font_family(&mut self, family: Option<String>) {
        self.font_family = family;
    }

    /// Get the typography link.
    pub fn typography_link(&self) -> Option<&crate::typography::TypographyLink> {
        self.typography_link.as_ref()
    }

    /// Set the typography link.
    pub fn set_typography_link(&mut self, link: Option<crate::typography::TypographyLink>) {
        self.typography_link = link;
    }

    /// Get the set of known icon files.
    pub fn known_files(&self) -> &HashSet<PathBuf> {
        &self.known_files
    }

    /// Add a known icon file.
    pub fn add_known_file(&mut self, path: PathBuf) {
        self.known_files.insert(path);
    }

    /// Get the masked icon cache.
    pub fn masked(&self) -> &crate::icons::MaskedCache {
        &self.masked
    }

    /// Get mutable access to the masked icon cache.
    pub fn masked_mut(&mut self) -> &mut crate::icons::MaskedCache {
        &mut self.masked
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Tests should verify ViewState through the app lifecycle:
    // theme changes, view memory persistence, appearance updates.
    // See compass-ui integration tests and LauncherApp test suite.
}
