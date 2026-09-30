//! Which windows belong to which application.
//!
//! Ports `XdgApplication::matchesWindowClass`
//! (`src/server/src/services/app-service/xdg/xdg-app.hpp`) and
//! `WindowManager::findAppWindows`
//! (`src/server/src/services/window-manager/window-manager.cpp`). Everything
//! `AbstractAppRuntime` offers — is this running, activate it, quit it — starts
//! by answering this question, and answering it wrongly quits the wrong
//! program.
//!
//! # The normalisation is odder than it looks
//!
//! ```cpp
//! auto normalizeClass = [](const QString &s) { return s.toLower().remove(".desktop"); };
//! ```
//!
//! `QString::remove` removes **every** occurrence, not a trailing one, so
//! `org.desktop.Foo.desktop` normalises to `orgfoo` rather than
//! `org.desktop.foo`. That is reproduced: a launcher that matched differently
//! from the engine it replaces would activate a different window.
//!
//! # Two ways to match, and the second is a guess
//!
//! `findAppWindows` matches a window when the application's
//! `matchesWindowClass` accepts its `WM_CLASS` **or** when the application's
//! display name equals the window's *title*, case-insensitively. The second is
//! a heuristic for applications that set no `StartupWMClass`, and it is a
//! guess: a text editor showing a file called "Firefox" would match Firefox.
//! Reproduced, because the C++ acts on it.

/// Lower-cases and removes every `.desktop`, as the C++ `normalizeClass` does.
#[must_use]
pub fn normalize_class(value: &str) -> String {
    value.to_lowercase().replace(".desktop", "")
}

/// What identifies an application for window matching.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppIdentity {
    /// The desktop file id, e.g. `firefox.desktop`.
    pub desktop_id: String,
    /// `StartupWMClass`, when the entry declares one.
    pub startup_wm_class: Option<String>,
    /// What the launcher shows, used by the title heuristic.
    pub display_name: String,
}

impl AppIdentity {
    /// Whether `wm_class` names this application.
    ///
    /// `StartupWMClass` first, then the desktop id — both normalised.
    #[must_use]
    pub fn matches_window_class(&self, wm_class: &str) -> bool {
        let target = normalize_class(wm_class);

        if let Some(declared) = &self.startup_wm_class
            && normalize_class(declared) == target
        {
            return true;
        }

        normalize_class(&self.desktop_id) == target
    }

    /// Whether a window with this class and title belongs to this application.
    ///
    /// The title arm is the C++'s second condition, and is a guess — see the
    /// module docs.
    #[must_use]
    pub fn matches_window(&self, wm_class: &str, title: &str) -> bool {
        self.matches_window_class(wm_class) || self.display_name.eq_ignore_ascii_case(title)
    }
}

/// The indices of `windows` that belong to `app`, in order.
///
/// Takes `(wm_class, title)` pairs rather than a window type so that this
/// crate keeps knowing nothing about any window manager.
#[must_use]
pub fn matching_windows<'a>(
    app: &AppIdentity,
    windows: impl IntoIterator<Item = (&'a str, &'a str)>,
) -> Vec<usize> {
    windows
        .into_iter()
        .enumerate()
        .filter_map(|(index, (class, title))| app.matches_window(class, title).then_some(index))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(desktop_id: &str, wm_class: Option<&str>, display_name: &str) -> AppIdentity {
        AppIdentity {
            desktop_id: desktop_id.to_owned(),
            startup_wm_class: wm_class.map(ToOwned::to_owned),
            display_name: display_name.to_owned(),
        }
    }

    #[test]
    fn a_startup_wm_class_matches_case_insensitively() {
        let firefox = app("firefox.desktop", Some("firefox"), "Firefox");
        assert!(firefox.matches_window_class("firefox"));
        assert!(firefox.matches_window_class("Firefox"));
        assert!(firefox.matches_window_class("FIREFOX"));
        assert!(!firefox.matches_window_class("chromium"));
    }

    #[test]
    fn the_desktop_id_matches_when_there_is_no_startup_class() {
        let app = app("org.gnome.Nautilus.desktop", None, "Files");
        assert!(
            app.matches_window_class("org.gnome.Nautilus"),
            "the id normalises to the class"
        );
        assert!(app.matches_window_class("org.gnome.nautilus.desktop"));
        assert!(!app.matches_window_class("nautilus"));
    }

    #[test]
    fn every_dot_desktop_is_removed_not_just_the_last() {
        // The quirk. `org.desktop.Foo.desktop` becomes `orgfoo`, so a window
        // class of `org.Foo` matches it -- in both engines.
        assert_eq!(normalize_class("org.desktop.Foo.desktop"), "org.foo");
        assert_eq!(normalize_class("A.desktop.B.desktop"), "a.b");

        let odd = app("org.desktop.Foo.desktop", None, "Foo");
        assert!(
            odd.matches_window_class("org.Foo"),
            "the normalisation eats the middle `.desktop` too"
        );
    }

    #[test]
    fn a_declared_class_that_does_not_match_does_not_stop_the_id_being_tried() {
        // `if (cl && ...) return true; return normalizeClass(id()) == target;`
        // The two must differ for this to check anything -- with
        // `spotify.desktop` and `Spotify` they normalise to the same string,
        // and an early return would pass unnoticed.
        let nautilus = app("org.gnome.Nautilus.desktop", Some("nautilus-alt"), "Files");
        assert!(
            nautilus.matches_window_class("nautilus-alt"),
            "the declared class matches"
        );
        assert!(
            nautilus.matches_window_class("org.gnome.Nautilus"),
            "and the id is still tried when it does not"
        );
    }

    #[test]
    fn the_title_heuristic_matches_the_display_name() {
        // The C++'s second condition, guess and all.
        let app = app("code.desktop", Some("Code"), "Visual Studio Code");
        assert!(app.matches_window("something-else", "Visual Studio Code"));
        assert!(app.matches_window("something-else", "visual studio code"));
        assert!(!app.matches_window("something-else", "Visual Studio"));
    }

    #[test]
    fn a_window_with_no_class_and_no_matching_title_belongs_to_nobody() {
        let app = app("firefox.desktop", Some("firefox"), "Firefox");
        assert!(!app.matches_window("", ""));
        assert!(
            !app.matches_window("", "Some Document"),
            "an empty class must not match an empty StartupWMClass check"
        );
    }

    #[test]
    fn matching_windows_answers_in_order_and_skips_the_rest() {
        let firefox = app("firefox.desktop", Some("firefox"), "Firefox");
        let windows = vec![
            ("chromium", "Something"),
            ("firefox", "A page"),
            ("Firefox", "Another page"),
            ("kitty", "Firefox"),
        ];
        assert_eq!(
            matching_windows(&firefox, windows.clone()),
            vec![1, 2, 3],
            "the last one matches by title, which is the heuristic the C++ has"
        );

        let none = app("nothing.desktop", Some("nothing"), "Nothing");
        assert!(matching_windows(&none, windows).is_empty());
    }
}
