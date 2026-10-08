//! The launcher's presentation state: which theme and palette it draws with,
//! the desktop's font, and what views remember between openings.
//!
//! This is the first piece split out of [`crate::app::LauncherApp`] (#258,
//! #271). None of it decides what the launcher shows; it decides how. The
//! fields are the crate's to read and write directly: the decisions about them
//! stay in the `app` modules that make them, as before.

use std::collections::HashSet;
use std::path::PathBuf;

use crate::design::Appearance;

/// How the launcher draws, rather than what it shows.
pub(crate) struct ViewState {
    /// Curated theme (#153).
    pub(crate) theme_choice: crate::theme::Theme,
    /// Where Set Theme looks for theme files.
    pub(crate) theme_dirs: Vec<PathBuf>,
    /// The theme to put back when a live preview is cancelled (#153).
    pub(crate) theme_preview: Option<crate::theme::Theme>,
    /// What views remember between openings.
    pub(crate) view_memory: crate::view_memory::ViewMemory,
    /// Which palette to draw with. See `LauncherApp::theme`.
    pub(crate) appearance: Appearance,
    /// Where later appearance changes arrive. See `AppFlags::appearance_link`.
    pub(crate) appearance_link: Option<crate::appearance::AppearanceLink>,
    /// The desktop's interface font family.
    ///
    /// `None` is the historic hard-coded `Cantarell` path. `Some` is whatever
    /// `org.gnome.desktop.interface font-name` reported, parsed to a family.
    /// See `crate::typography`.
    pub(crate) font_family: Option<String>,
    /// Where later font changes arrive. See `AppFlags::typography_link`.
    pub(crate) typography_link: Option<crate::typography::TypographyLink>,
    /// Masked images, drawn once.
    pub(crate) masked: crate::icons::MaskedCache,
    /// Extension icon files seen to exist, so a draw does not stat them.
    pub(crate) known_files: HashSet<PathBuf>,
}

impl Default for ViewState {
    /// What a launcher starts with before `AppFlags` and the desktop say
    /// otherwise: the system theme, the light palette, the historic font.
    fn default() -> Self {
        Self {
            theme_choice: crate::theme::Theme::System,
            theme_dirs: Vec::new(),
            theme_preview: None,
            view_memory: crate::view_memory::ViewMemory::default(),
            appearance: Appearance::Light,
            appearance_link: None,
            font_family: None,
            typography_link: None,
            masked: crate::icons::MaskedCache::default(),
            known_files: HashSet::new(),
        }
    }
}
