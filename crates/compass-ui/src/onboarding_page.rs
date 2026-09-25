//! The first-run flow's state in the launcher: where the flow is, the themes
//! the "Make it your own" step offers, and where finishing is recorded.
//!
//! `compass_core::onboarding` decides the steps and the file; this is what the
//! launcher's page holds around it.

use std::path::PathBuf;

use compass_core::onboarding::Flow;

use crate::theme::Theme;

/// One entry of the theme dropdown: a theme, shown by its title.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThemeOption(pub Theme);

impl std::fmt::Display for ThemeOption {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0.title())
    }
}

/// The page's state.
#[derive(Debug, Clone)]
pub struct OnboardingPage {
    /// Where the flow is.
    pub flow: Flow,
    /// The themes offered: the curated ones, then the theme files.
    pub themes: Vec<ThemeOption>,
    /// Where finishing is recorded (`onboarding.json`).
    pub state_path: PathBuf,
    /// Why something did not happen (a theme not kept, a link not opened).
    pub notice: Option<String>,
}

impl OnboardingPage {
    /// The flow from its first step, offering the curated themes and
    /// `files`; Linux has no permissions step.
    #[must_use]
    pub fn new(state_path: PathBuf, files: Vec<Theme>) -> Self {
        let mut themes: Vec<ThemeOption> = Theme::ALL.into_iter().map(ThemeOption).collect();
        themes.extend(files.into_iter().map(ThemeOption));
        Self {
            flow: Flow::new(false),
            themes,
            state_path,
            notice: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_dropdown_offers_the_curated_themes_by_title() {
        let page = OnboardingPage::new(PathBuf::from("/nonexistent"), Vec::new());
        assert_eq!(page.themes.len(), Theme::ALL.len());
        assert_eq!(page.themes[0].to_string(), Theme::ALL[0].title());
        assert_eq!(page.flow.count(), 3, "no permissions step on Linux");
    }
}
