//! Looking an application up: by id, by window class, or by name.
//!
//! Ports the lookups `AppService` delegates to `XdgAppDatabase`
//! (`src/server/src/services/app-service/`), over the [`crate::AppIndex`] that
//! already reads the desktop entries. Only the lookups: launching, the file
//! browser and the terminal are process work that belongs to whoever owns the
//! session, and the MIME-association search needs a shared-mime-info database
//! this crate does not have yet — `findOpeners` and `findDefaultOpener` are
//! still C++-only, and PARITY.md says so.

use crate::app_windows::AppIdentity;
use crate::{AppIndex, AppItem};

/// The lookups, over one index.
#[derive(Debug, Clone, Copy)]
pub struct AppService<'a> {
    index: &'a AppIndex,
}

impl<'a> AppService<'a> {
    /// Looks applications up in `index`.
    #[must_use]
    pub const fn new(index: &'a AppIndex) -> Self {
        Self { index }
    }

    /// `XdgAppDatabase::findById`.
    ///
    /// Tries the id as given and then with `.desktop` appended, so `firefox`
    /// finds `firefox.desktop`. Actions are not addressable this way: the map
    /// the C++ searches is keyed by desktop file id, and an action lives inside
    /// one.
    #[must_use]
    pub fn find_by_id(&self, id: &str) -> Option<&'a AppItem> {
        self.applications()
            .find(|item| item.desktop_id() == id)
            .or_else(|| {
                let suffixed = format!("{id}.desktop");
                self.applications()
                    .find(|item| item.desktop_id() == suffixed)
            })
    }

    /// `XdgAppDatabase::findByClass`.
    ///
    /// The first application whose identity matches, in index order — the C++
    /// `find_if` over `m_apps` with the same "we might need to use a map to
    /// speed that up" linear scan.
    #[must_use]
    pub fn find_by_class(&self, wm_class: &str) -> Option<&'a AppItem> {
        self.applications()
            .find(|item| identity(item).matches_window_class(wm_class))
    }

    /// The window classes of every terminal emulator (`isTerminalEmulator()`,
    /// the `TerminalEmulator` category), normalised as
    /// [`normalize_class`](crate::app_windows::normalize_class) does: each
    /// one's `StartupWMClass` when it declares one, and its desktop id either
    /// way, the same two things [`Self::find_by_class`] matches. A paste into
    /// one of these needs Ctrl+Shift+V.
    #[must_use]
    pub fn terminal_window_classes(&self) -> Vec<String> {
        let mut classes: Vec<String> = self
            .applications()
            .filter(|item| item.categories().iter().any(|c| c == "TerminalEmulator"))
            .flat_map(|item| {
                let identity = identity(item);
                identity
                    .startup_wm_class
                    .as_deref()
                    .map(crate::app_windows::normalize_class)
                    .into_iter()
                    .chain(std::iter::once(crate::app_windows::normalize_class(
                        &identity.desktop_id,
                    )))
                    .collect::<Vec<_>>()
            })
            .collect();
        classes.sort_unstable();
        classes.dedup();
        classes
    }

    /// `AppService::find`: by id first, then by window class.
    ///
    /// The order matters for a target that is both, and an id is the more
    /// specific claim.
    #[must_use]
    pub fn find(&self, target: &str) -> Option<&'a AppItem> {
        self.find_by_id(target)
            .or_else(|| self.find_by_class(target))
    }

    /// `AppService::list`.
    ///
    /// `sortAlphabetically` sorts by display name, case-insensitively, as
    /// `QString::compare(..., Qt::CaseInsensitive)` does. Otherwise the index's
    /// own order is kept.
    #[must_use]
    pub fn list(&self, sort_alphabetically: bool) -> Vec<&'a AppItem> {
        let mut apps: Vec<&AppItem> = self.applications().collect();
        if sort_alphabetically {
            apps.sort_by_key(|item| item.display_name().to_lowercase());
        }
        apps
    }

    /// `AppService::findCuratedOpeners`, over any list of applications.
    ///
    /// Drops every application whose *display name* has already been seen,
    /// keeping the first. Two entries called "Firefox" — a system one and a
    /// Flatpak — are one choice as far as the user is concerned, and the list
    /// is already in preference order.
    #[must_use]
    pub fn curated(apps: impl IntoIterator<Item = &'a AppItem>) -> Vec<&'a AppItem> {
        let mut seen: Vec<String> = Vec::new();
        let mut out = Vec::new();
        for app in apps {
            let name = app.display_name();
            if seen.contains(&name) {
                continue;
            }
            seen.push(name);
            out.push(app);
        }
        out
    }

    /// The index's applications, without the desktop actions.
    ///
    /// The C++ `m_apps` holds applications; actions hang off them and are not
    /// separate entries, so a lookup that returned one would answer with
    /// something `AppService` cannot launch on its own.
    fn applications(&self) -> impl Iterator<Item = &'a AppItem> {
        self.index.items().iter().filter(|item| !item.is_action())
    }
}

/// The identity `matchesWindowClass` uses.
#[must_use]
pub fn identity(item: &AppItem) -> AppIdentity {
    AppIdentity {
        desktop_id: item.desktop_id().to_owned(),
        startup_wm_class: item.entry().startup_wm_class().map(str::to_owned),
        display_name: item.display_name(),
    }
}
