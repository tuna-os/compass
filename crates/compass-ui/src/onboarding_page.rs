//! The first-run flow's state in the launcher: where the flow is, the themes
//! the "Make it your own" step offers, and where finishing is recorded.
//!
//! `compass_core::onboarding` decides the steps and the file; this is what the
//! launcher's page holds around it.

use std::path::PathBuf;

use compass_core::onboarding::{Extensions, Flow};

use crate::theme::Theme;

/// One entry of the theme dropdown: a theme, shown by its title.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThemeOption(pub Theme);

impl std::fmt::Display for ThemeOption {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0.title())
    }
}

/// A control on a step that the keyboard can reach.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Control {
    /// The theme dropdown.
    Theme,
    /// "Open Docs" for the hotkey.
    OpenDocs,
    /// A recommended extension's Install, by its position.
    Install(usize),
    /// The last step's GitHub link.
    GitHub,
    /// Back.
    Back,
    /// Continue, or Finish on the last step.
    Continue,
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
    /// The extensions step's recommendations and their installs.
    pub extensions: Extensions,
    /// The control with the keyboard, if one has it.
    pub focused: Option<Control>,
    /// The theme highlighted in the open theme dropdown.
    pub menu: Option<usize>,
}

impl OnboardingPage {
    /// The flow from its first step, offering the curated themes and
    /// `files`, and recommending the extensions `installed` does not know
    /// by id; Linux has no permissions step.
    #[must_use]
    pub fn new(state_path: PathBuf, files: Vec<Theme>, installed: impl Fn(&str) -> bool) -> Self {
        let mut themes: Vec<ThemeOption> = Theme::ALL.into_iter().map(ThemeOption).collect();
        themes.extend(files.into_iter().map(ThemeOption));
        Self {
            flow: Flow::new(false),
            themes,
            state_path,
            notice: None,
            extensions: Extensions::new(installed),
            focused: None,
            menu: None,
        }
    }

    /// The step's controls in the order they are drawn: its own, then Back
    /// and Continue. An Install that cannot be pressed is not one.
    #[must_use]
    pub fn controls(&self) -> Vec<Control> {
        use compass_core::onboarding::{Install, RECOMMENDED_EXTENSIONS, Step};
        let mut controls = match self.flow.step() {
            Step::Personalize => vec![Control::Theme, Control::OpenDocs],
            Step::Extensions => (0..RECOMMENDED_EXTENSIONS.len())
                .filter(|&index| {
                    self.extensions
                        .state(index)
                        .unwrap_or(Install::Available)
                        .can_install()
                })
                .map(Control::Install)
                .collect(),
            Step::Complete => vec![Control::GitHub],
            Step::Welcome | Step::Permissions => Vec::new(),
        };
        if self.flow.can_go_back() {
            controls.push(Control::Back);
        }
        controls.push(Control::Continue);
        controls
    }

    /// Tab or Shift+Tab.
    pub fn move_focus(&mut self, forward: bool) {
        self.focused = crate::focus::step(&self.controls(), self.focused.as_ref(), forward);
        self.menu = None;
    }

    /// Another step is on show: nothing on it has the keyboard yet.
    pub fn step_changed(&mut self) {
        self.focused = None;
        self.menu = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_dropdown_offers_the_curated_themes_by_title() {
        let page = OnboardingPage::new(PathBuf::from("/nonexistent"), Vec::new(), |_| false);
        assert_eq!(page.themes.len(), Theme::ALL.len());
        assert_eq!(page.themes[0].to_string(), Theme::ALL[0].title());
        assert_eq!(page.flow.count(), 4, "no permissions step on Linux");
    }

    #[test]
    fn tab_reaches_each_steps_controls_then_back_and_continue() {
        let mut page = OnboardingPage::new(PathBuf::from("/nonexistent"), Vec::new(), |_| false);
        assert_eq!(page.controls(), [Control::Continue], "the welcome step");
        let _ = page.flow.advance();
        assert_eq!(
            page.controls(),
            [
                Control::Theme,
                Control::OpenDocs,
                Control::Back,
                Control::Continue
            ]
        );
        page.move_focus(true);
        assert_eq!(page.focused, Some(Control::Theme));
        page.move_focus(false);
        page.move_focus(false);
        assert_eq!(page.focused, Some(Control::Continue));
        let _ = page.flow.advance();
        page.step_changed();
        let installs = page
            .controls()
            .into_iter()
            .filter(|control| matches!(control, Control::Install(_)))
            .count();
        assert_eq!(
            installs,
            compass_core::onboarding::RECOMMENDED_EXTENSIONS.len()
        );
    }
}
