//! Shortcuts (quicklinks): the engine's store, and what opening one does.
//!
//! The list and its file are `compass_core::shortcut_service`; this is where
//! the engine keeps it, how it comes across from Vicinae, and the parts of
//! `shortcut-actions.hpp` that need the machine — the clipboard for
//! `{clipboard}`, the application database for the opener and the default
//! icon.
//!
//! Compass keeps its own file, `compass-shortcuts.json`, beside the rest of
//! its data (ADR-0017 decision 3). Vicinae's `shortcuts/shortcuts.json` has
//! the same shape, so bringing it across is a copy, done once, before the
//! first list is built: the first start that finds no Compass file and a
//! Vicinae one takes the Vicinae one.

use std::path::{Path, PathBuf};

use compass_core::image_url::{ImageUrl, ImageUrlType};
use compass_core::shortcut_service::{CachedShortcut, ShortcutService};
use compass_ipc::ShortcutEntry;
use compass_worker_host::application_service::{Application, Apps};

/// Compass's shortcut file, in its data directory.
pub const FILE_NAME: &str = "compass-shortcuts.json";

/// Vicinae's shortcut file, relative to the shared data directory.
pub const VICINAE_FILE: &str = "shortcuts/shortcuts.json";

/// Where Compass keeps its shortcuts: `$XDG_DATA_HOME/vicinae`, beside the
/// clipboard history and the launch history.
#[must_use]
pub fn data_dir() -> Option<PathBuf> {
    compass_core::xdg_dirs::data_home().map(|home| home.join("vicinae"))
}

/// Opens the store in `dir`, bringing Vicinae's shortcuts across first when
/// Compass has none of its own yet.
#[must_use]
pub fn open(dir: &Path) -> ShortcutService {
    let path = dir.join(FILE_NAME);
    import_from_vicinae(&path, &dir.join(VICINAE_FILE));
    ShortcutService::open(&path)
}

/// Copies Vicinae's shortcut file to `compass` when `compass` does not exist
/// and `vicinae` holds a list that parses. Returns whether it did.
///
/// A Vicinae file that does not parse is left alone and Compass starts empty,
/// rather than copying a file its own store would then read as nothing and
/// overwrite on the first save.
pub fn import_from_vicinae(compass: &Path, vicinae: &Path) -> bool {
    if compass.exists() {
        return false;
    }
    let Ok(text) = std::fs::read_to_string(vicinae) else {
        return false;
    };
    if serde_json::from_str::<Vec<compass_core::shortcut_store::SerializedShortcut>>(&text).is_err()
    {
        tracing::warn!(path = %vicinae.display(), "Vicinae's shortcuts do not parse; not importing them");
        return false;
    }
    if let Some(parent) = compass.parent()
        && std::fs::create_dir_all(parent).is_err()
    {
        return false;
    }
    match std::fs::write(compass, text) {
        Ok(()) => {
            tracing::info!(from = %vicinae.display(), "imported Vicinae's shortcuts");
            true
        }
        Err(error) => {
            tracing::warn!(%error, "could not import Vicinae's shortcuts");
            false
        }
    }
}

/// A shortcut as the wire carries it.
#[must_use]
pub fn entry(shortcut: &CachedShortcut) -> ShortcutEntry {
    ShortcutEntry {
        id: shortcut.id.clone(),
        name: shortcut.name.clone(),
        icon: shortcut.icon.clone(),
        url: shortcut.link.raw.clone(),
        app: shortcut.app.clone(),
        open_count: shortcut.open_count,
        created_at: shortcut.created_at,
        updated_at: shortcut.updated_at,
        last_used_at: shortcut.last_opened_at,
    }
}

/// The current time in Unix seconds.
#[must_use]
pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_secs())
}

/// The reserved placeholders' values for one expansion.
///
/// The clipboard is read before expanding (reading it is a D-Bus call), and
/// only when the link has `{clipboard}`.
#[derive(Debug, Default)]
pub struct Reserved {
    /// The clipboard's text, when the link needs it and it could be read.
    pub clipboard: Option<String>,
}

impl compass_core::shortcut::Reserved for Reserved {
    fn clipboard(&self) -> Option<String> {
        self.clipboard.clone()
    }

    // Reading the focused application's selection needs the selection
    // service, which is not ported (PARITY, Shortcuts).
    fn selection(&self) -> Option<String> {
        None
    }

    fn uuid(&self) -> String {
        uuid::Uuid::new_v4().hyphenated().to_string()
    }
}

/// Whether a link reads the clipboard when expanded.
#[must_use]
pub fn needs_clipboard(shortcut: &CachedShortcut) -> bool {
    shortcut
        .link
        .placeholders
        .iter()
        .any(|placeholder| placeholder.id == "clipboard")
}

/// The browser, as `XdgAppDatabase::webBrowser` finds it: what opens
/// `https`, then `http`, then HTML.
fn web_browser(apps: &dyn Apps) -> Option<Application> {
    ["https://", "http://", "text/html"]
        .into_iter()
        .find_map(|target| apps.default_opener(target))
}

/// The application that opens `target` for a shortcut whose app is `app_id`,
/// as `ShortcutService::resolveApp` chooses it: the named application, or
/// for `default` the target's default opener, else the browser.
#[must_use]
pub fn resolve_app(apps: &dyn Apps, app_id: &str, target: &str) -> Option<Application> {
    if app_id != compass_core::shortcut::DEFAULT_APP_ID {
        return apps.by_id(app_id);
    }
    apps.default_opener(target).or_else(|| web_browser(apps))
}

/// What the `default` icon stands for when a shortcut with `url` is saved:
/// the site's favicon for an `http*` link, else the icon of the application
/// that opens it, else the built-in link glyph. `ShortcutFormViewHost`
/// resolves it the same way, and stores the result rather than the word.
#[must_use]
pub fn resolve_default_icon(apps: &dyn Apps, url: &str) -> String {
    let (scheme, host) = url::Url::parse(url).map_or_else(
        |_| (String::new(), String::new()),
        |parsed| {
            (
                parsed.scheme().to_owned(),
                parsed.host_str().unwrap_or_default().to_owned(),
            )
        },
    );
    let opener_icon = apps
        .default_opener(url)
        .map(|app| app.icon)
        .filter(|icon| !icon.is_empty());
    match compass_core::shortcut_form::default_icon(opener_icon.as_deref(), &scheme, &host) {
        compass_core::shortcut_form::DefaultIcon::Favicon { host } => {
            ImageUrl::new(ImageUrlType::Favicon, host)
                .with_fallback(&ImageUrl::builtin("image"))
                .to_url()
        }
        compass_core::shortcut_form::DefaultIcon::Opener { icon } => {
            let kind = if icon.starts_with('/') {
                ImageUrlType::Local
            } else {
                ImageUrlType::System
            };
            ImageUrl::new(kind, icon).to_url()
        }
        compass_core::shortcut_form::DefaultIcon::BuiltinLink => ImageUrl::builtin("link").to_url(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Default)]
    struct FakeApps {
        openers: Vec<(&'static str, Application)>,
    }

    fn app(id: &str, icon: &str) -> Application {
        Application {
            id: id.to_owned(),
            name: id.to_owned(),
            icon: icon.to_owned(),
            path: String::new(),
        }
    }

    impl Apps for FakeApps {
        fn list(&self) -> Vec<Application> {
            self.openers.iter().map(|(_, app)| app.clone()).collect()
        }
        fn openers(&self, target: &str) -> Vec<Application> {
            self.default_opener(target).into_iter().collect()
        }
        fn default_opener(&self, target: &str) -> Option<Application> {
            self.openers
                .iter()
                .find(|(prefix, _)| target.starts_with(prefix))
                .map(|(_, app)| app.clone())
        }
        fn by_id(&self, id: &str) -> Option<Application> {
            self.list().into_iter().find(|app| app.id == id)
        }
        fn launch(&self, _app: &Application, _target: &str) {}
        fn show_in_file_browser(&self, _target: &str, _select: bool) {}
        fn run_in_terminal(
            &self,
            _cmdline: &[String],
            _options: &compass_worker_host::application_service::TerminalOptions,
        ) -> bool {
            false
        }
    }

    #[test]
    fn vicinae_shortcuts_come_across_once_and_only_when_they_parse() {
        let dir = tempfile::tempdir().unwrap();
        let vicinae = dir.path().join(VICINAE_FILE);
        std::fs::create_dir_all(vicinae.parent().unwrap()).unwrap();
        std::fs::write(
            &vicinae,
            r#"[{"id":"sct-aaaaaaaaaaaa","name":"Docs","icon":"icon://builtin/link",
                "url":"https://docs.rs/{crate}","app":"default","openCount":2,
                "createdAt":1,"updatedAt":2}]"#,
        )
        .unwrap();
        let service = open(dir.path());
        assert_eq!(service.shortcuts().len(), 1);
        assert_eq!(service.shortcuts()[0].name, "Docs");
        assert_eq!(service.shortcuts()[0].link.arguments[0].name, "crate");

        std::fs::write(&vicinae, "[]").unwrap();
        assert!(
            !import_from_vicinae(&dir.path().join(FILE_NAME), &vicinae),
            "Compass's own file wins from then on"
        );

        let other = tempfile::tempdir().unwrap();
        let broken = other.path().join(VICINAE_FILE);
        std::fs::create_dir_all(broken.parent().unwrap()).unwrap();
        std::fs::write(&broken, "{not json").unwrap();
        assert!(!import_from_vicinae(&other.path().join(FILE_NAME), &broken));
        assert!(open(other.path()).shortcuts().is_empty());
    }

    #[test]
    fn the_app_is_the_named_one_or_the_opener_or_the_browser() {
        let apps = FakeApps {
            openers: vec![
                ("https", app("firefox.desktop", "firefox")),
                ("mailto", app("thunderbird.desktop", "/opt/tb.png")),
            ],
        };
        assert_eq!(
            resolve_app(&apps, "thunderbird.desktop", "https://x").map(|a| a.id),
            Some("thunderbird.desktop".to_owned())
        );
        assert_eq!(
            resolve_app(&apps, "default", "mailto:a@b").map(|a| a.id),
            Some("thunderbird.desktop".to_owned())
        );
        assert_eq!(
            resolve_app(&apps, "default", "zzz:nothing").map(|a| a.id),
            Some("firefox.desktop".to_owned()),
            "nothing claims the scheme, so the browser opens it"
        );
        assert!(resolve_app(&apps, "gone.desktop", "https://x").is_none());
    }

    #[test]
    fn the_default_icon_is_the_favicon_then_the_opener_then_the_link_glyph() {
        let apps = FakeApps {
            openers: vec![
                ("https", app("firefox.desktop", "firefox")),
                ("mailto", app("thunderbird.desktop", "/opt/tb.png")),
            ],
        };
        assert_eq!(
            resolve_default_icon(&apps, "https://github.com/search?q={q}"),
            "icon://favicon/github.com?fallback=icon://omnicast/image"
        );
        assert_eq!(
            resolve_default_icon(&apps, "mailto:{to}"),
            "icon://local//opt/tb.png"
        );
        assert_eq!(
            resolve_default_icon(&apps, "{clipboard}"),
            "icon://omnicast/link"
        );
    }
}
