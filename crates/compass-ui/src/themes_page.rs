//! Set Theme: the themes in two sections, and the live preview.
//!
//! `compass_core::theme_picker` decides the sections and the filter, as
//! `ThemeListModel` does; the themes are Compass's curated ones
//! ([`crate::theme::Theme`]) rather than the C++'s theme files.

use compass_core::theme_picker::{self, AVAILABLE_SECTION, CURRENT_SECTION};

use crate::theme::Theme;

/// One row: the theme, and whether it starts its section.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThemeRow {
    /// The theme.
    pub theme: Theme,
    /// The heading above it, when it is the first of its section.
    pub heading: Option<&'static str>,
}

/// The view's state.
#[derive(Debug, Clone)]
pub struct ThemesPage {
    /// The search text.
    pub query: String,
    /// The theme in use when the view opened: its "current" row, and what
    /// leaving without choosing goes back to.
    pub configured: Theme,
    /// The rows: the current theme, then the others.
    pub rows: Vec<ThemeRow>,
    /// Position in `rows`.
    pub selected: usize,
    /// Why saving did not happen.
    pub notice: Option<String>,
}

fn picker_theme(theme: Theme) -> theme_picker::Theme {
    theme_picker::Theme {
        id: theme.name().to_owned(),
        name: theme.title().to_owned(),
        description: theme.description().to_owned(),
        icon: None,
        path: None,
    }
}

impl ThemesPage {
    /// The view over every theme, `configured` first.
    #[must_use]
    pub fn new(configured: Theme) -> Self {
        let mut page = Self {
            query: String::new(),
            configured,
            rows: Vec::new(),
            selected: 0,
            notice: None,
        };
        page.refilter();
        page
    }

    /// Recomputes the rows for the current text, back at the top.
    pub fn refilter(&mut self) {
        let themes: Vec<theme_picker::Theme> =
            Theme::ALL.iter().copied().map(picker_theme).collect();
        let list = theme_picker::split(&themes, &self.query, self.configured.name());
        let rows = |section: &[theme_picker::Theme], heading: &'static str| {
            section
                .iter()
                .enumerate()
                .filter_map(|(position, theme)| {
                    Theme::from_name(&theme.id).map(|theme| ThemeRow {
                        theme,
                        heading: (position == 0).then_some(heading),
                    })
                })
                .collect::<Vec<_>>()
        };
        self.rows = rows(&list.current, CURRENT_SECTION);
        self.rows.extend(rows(&list.available, AVAILABLE_SECTION));
        self.selected = 0;
    }

    /// The selected theme, if any.
    #[must_use]
    pub fn selected_theme(&self) -> Option<Theme> {
        self.rows.get(self.selected).map(|row| row.theme)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_configured_theme_is_its_own_section_and_the_filter_is_fuzzy() {
        let mut page = ThemesPage::new(Theme::Nord);
        assert_eq!(page.rows[0].theme, Theme::Nord);
        assert_eq!(page.rows[0].heading, Some(CURRENT_SECTION));
        assert_eq!(page.rows[1].heading, Some(AVAILABLE_SECTION));
        assert_eq!(page.rows.len(), Theme::ALL.len());
        page.query = "tokyo nigt".into();
        page.refilter();
        assert_eq!(page.selected_theme(), Some(Theme::TokyoNight));
        assert_eq!(page.rows[0].heading, Some(AVAILABLE_SECTION));
    }
}
