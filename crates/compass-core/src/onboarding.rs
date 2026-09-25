//! The first-run flow: whether it is due, its steps, and the record that it
//! was finished.
//!
//! Ports `OnboardingWindow` (`ui/windows/onboarding-window.*`) and the step
//! logic of `OnboardingWindow.qml`. The C++ shows the flow at server start
//! when `onboarding.json` in its state directory records a version older than
//! [`VERSION`] (or nothing), and only in a build with `ENABLE_ONBOARDING`,
//! which is on by default. Compass reads and writes the same file, so a person
//! who finished it under either engine is not asked again; its switch is
//! [`DISABLE_ENV`], set by the harnesses that must see the bare launcher.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// `ONBOARDING_VERSION`: bumping it shows the flow again to everyone.
pub const VERSION: u32 = 1;

/// The state file's name, in `$XDG_STATE_HOME/vicinae`.
pub const FILE_NAME: &str = "onboarding.json";

/// Set (to anything but `0` or empty) to leave the flow out, as a build
/// without `ENABLE_ONBOARDING` does.
pub const DISABLE_ENV: &str = "COMPASS_NO_ONBOARDING";

/// The window's title and the first step's heading.
pub const TITLE: &str = "Welcome to Vicinae";

/// What `onboarding.json` holds (`OnboardingState`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct State {
    /// The version of the flow last finished; 0 for never.
    pub version: u32,
    /// When, as an ISO 8601 UTC time.
    pub completed_at: String,
}

/// `$XDG_STATE_HOME/vicinae/onboarding.json`, falling back to
/// `~/.local/state`, as `Omnicast::stateDir()`.
#[must_use]
pub fn default_path() -> Option<PathBuf> {
    let state = match std::env::var_os("XDG_STATE_HOME") {
        Some(value) if !value.is_empty() => PathBuf::from(value),
        _ => crate::xdg_dirs::home_dir()?.join(".local/state"),
    };
    Some(state.join("vicinae").join(FILE_NAME))
}

/// The version recorded at `path`; 0 when there is no file or it cannot be
/// read, as `completedVersion`.
#[must_use]
pub fn completed_version(path: &Path) -> u32 {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str::<State>(&text).ok())
        .map_or(0, |state| state.version)
}

/// `OnboardingWindow::shouldShow`: the flow is due when what was finished is
/// older than [`VERSION`], unless `disabled`.
#[must_use]
pub fn should_show(path: &Path, disabled: bool) -> bool {
    !disabled && completed_version(path) < VERSION
}

/// Whether [`DISABLE_ENV`]'s value turns the flow off.
#[must_use]
pub fn disabled_by(value: Option<&std::ffi::OsStr>) -> bool {
    value.is_some_and(|value| !value.is_empty() && value != "0")
}

/// `markCompleted`: records [`VERSION`] as finished at `completed_at`,
/// creating the state directory if it is not there.
///
/// # Errors
///
/// When the directory cannot be created or the file written.
pub fn mark_completed(path: &Path, completed_at: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let state = State {
        version: VERSION,
        completed_at: completed_at.to_owned(),
    };
    let text = serde_json::to_string(&state).map_err(std::io::Error::other)?;
    std::fs::write(path, text)
}

/// One step of the flow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// "Welcome to Vicinae".
    Welcome,
    /// macOS's permissions; never offered on Linux.
    Permissions,
    /// "Make it your own": the theme and the global hotkey.
    Personalize,
    /// "Setup complete".
    Complete,
}

impl Step {
    /// The step's heading.
    #[must_use]
    pub const fn heading(self) -> &'static str {
        match self {
            Self::Welcome => TITLE,
            Self::Permissions => "Permissions",
            Self::Personalize => "Make it your own",
            Self::Complete => "Setup complete",
        }
    }

    /// The line under the heading. `shortcuts` is whether the platform binds
    /// global shortcuts itself, which changes the last step's sentence.
    #[must_use]
    pub const fn subtitle(self, shortcuts: bool) -> &'static str {
        match self {
            Self::Welcome => "Let's set it up. It only takes a minute.",
            Self::Permissions => {
                "Vicinae needs additional permissions in order to make the best of your Mac."
            }
            Self::Personalize => "You will be able to change these settings later.",
            Self::Complete if shortcuts => "Vicinae is running. Open the launcher with:",
            Self::Complete => {
                "Vicinae is running. Bind a key to \"vicinae toggle\" to open it from anywhere."
            }
        }
    }
}

/// Where Continue took the flow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Advance {
    /// To the next step.
    Next,
    /// Past the last one: the flow is finished.
    Finish,
}

/// The flow's position, as `root.step` over `stepCount` steps.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Flow {
    steps: Vec<Step>,
    current: usize,
}

impl Flow {
    /// The steps, with the permissions page only where the platform has
    /// permissions to ask for (`permissionsAvailable`, macOS).
    #[must_use]
    pub fn new(permissions: bool) -> Self {
        let steps = if permissions {
            vec![
                Step::Welcome,
                Step::Permissions,
                Step::Personalize,
                Step::Complete,
            ]
        } else {
            vec![Step::Welcome, Step::Personalize, Step::Complete]
        };
        Self { steps, current: 0 }
    }

    /// The step on screen.
    #[must_use]
    pub fn step(&self) -> Step {
        self.steps[self.current]
    }

    /// Its position.
    #[must_use]
    pub fn position(&self) -> usize {
        self.current
    }

    /// How many steps there are (the dots).
    #[must_use]
    pub fn count(&self) -> usize {
        self.steps.len()
    }

    /// Whether Back is shown.
    #[must_use]
    pub fn can_go_back(&self) -> bool {
        self.current > 0
    }

    /// The primary button's label: Finish on the last step, else Continue.
    #[must_use]
    pub fn primary_label(&self) -> &'static str {
        if self.current + 1 == self.steps.len() {
            "Finish"
        } else {
            "Continue"
        }
    }

    /// `advance()`: the next step, or the end.
    pub fn advance(&mut self) -> Advance {
        if self.current + 1 == self.steps.len() {
            return Advance::Finish;
        }
        self.current += 1;
        Advance::Next
    }

    /// `goBack()`.
    pub fn back(&mut self) {
        self.current = self.current.saturating_sub(1);
    }

    /// A dot was clicked: straight to that step.
    pub fn jump(&mut self, position: usize) {
        if position < self.steps.len() {
            self.current = position;
        }
    }
}

/// The documentation the hotkey row links to where the platform does not
/// bind global shortcuts.
pub const HOTKEY_DOCS_URL: &str =
    "https://docs.vicinae.com/faq#how-to-set-a-keyboard-shortcut-to-open-vicinae";
/// The last step's GitHub button.
pub const GITHUB_URL: &str = "https://github.com/vicinaehq/vicinae";
/// The last step's Sponsor button.
pub const SPONSOR_URL: &str = "https://github.com/sponsors/vicinaehq";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_is_due_until_the_current_version_is_recorded() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vicinae").join(FILE_NAME);
        assert!(should_show(&path, false), "no file is never finished");
        assert!(!should_show(&path, true), "switched off");

        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "not json").unwrap();
        assert!(should_show(&path, false), "unreadable counts as never");
        std::fs::write(&path, r#"{"version":0,"completedAt":""}"#).unwrap();
        assert!(should_show(&path, false), "an older version");

        std::fs::remove_file(&path).unwrap();
        mark_completed(&path, "2026-09-25T10:00:00Z").unwrap();
        assert!(!should_show(&path, false));
        let written: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(
            written,
            serde_json::json!({"version": 1, "completedAt": "2026-09-25T10:00:00Z"}),
            "the C++'s file, field for field"
        );
    }

    #[test]
    fn the_cpps_own_file_is_read() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE_NAME);
        std::fs::write(
            &path,
            r#"{"version":1,"completedAt":"2026-01-02T03:04:05Z"}"#,
        )
        .unwrap();
        assert_eq!(completed_version(&path), 1);
        assert!(!should_show(&path, false));
    }

    #[test]
    fn linux_has_three_steps_and_continue_finishes_on_the_last() {
        let mut flow = Flow::new(false);
        assert_eq!(flow.count(), 3);
        assert_eq!(flow.step(), Step::Welcome);
        assert!(!flow.can_go_back());
        assert_eq!(flow.primary_label(), "Continue");
        assert_eq!(flow.advance(), Advance::Next);
        assert_eq!(flow.step(), Step::Personalize);
        assert_eq!(flow.advance(), Advance::Next);
        assert_eq!(flow.step(), Step::Complete);
        assert_eq!(flow.primary_label(), "Finish");
        assert_eq!(flow.advance(), Advance::Finish);
        assert_eq!(flow.step(), Step::Complete, "finishing stays put");
        flow.back();
        assert_eq!(flow.step(), Step::Personalize);
        flow.jump(0);
        assert_eq!(flow.step(), Step::Welcome);
        flow.jump(9);
        assert_eq!(flow.step(), Step::Welcome, "no such dot");
        flow.back();
        assert_eq!(flow.position(), 0);
    }

    #[test]
    fn the_permissions_step_is_macos_only() {
        let mut flow = Flow::new(true);
        assert_eq!(flow.count(), 4);
        flow.advance();
        assert_eq!(flow.step(), Step::Permissions);
    }

    #[test]
    fn the_switch_reads_like_a_boolean_environment_variable() {
        use std::ffi::OsStr;
        assert!(!disabled_by(None));
        assert!(!disabled_by(Some(OsStr::new(""))));
        assert!(!disabled_by(Some(OsStr::new("0"))));
        assert!(disabled_by(Some(OsStr::new("1"))));
    }

    #[test]
    fn the_last_step_says_how_to_open_the_launcher() {
        assert!(Step::Complete.subtitle(false).contains("vicinae toggle"));
        assert_eq!(
            Step::Complete.subtitle(true),
            "Vicinae is running. Open the launcher with:"
        );
    }
}
