//! Following `compass.json` while Compass runs, and saying what is wrong
//! with it.
//!
//! A hand edit, `compass theme set` or `compass config set` used to change
//! nothing until the next start. Both processes now watch the file: the
//! engine re-applies what it holds ([`crate::serve::settings::apply_all`]),
//! and the launcher window re-reads its settings (`compass_ui`'s
//! `ConfigLink`). The watch is on the file's directory rather than the file,
//! because editors save by writing a new file and renaming it over the old
//! one, which a watch on the old inode would not see.
//!
//! [`check`] is what the engine logs at start and after each change, and what
//! `compass doctor` reports: the problems `compass_core` finds on its own
//! (wrongly typed values, unknown keys), plus the names only this crate can
//! check because the UI owns them (themes, presets).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use compass_core::Config;
use compass_core::config_issues::ConfigIssue;
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::RwLock;

use crate::serve::EngineState;

/// How long the file has to be quiet before it is read: an editor's save is
/// several events (write, rename, attribute change).
pub const DEBOUNCE: Duration = Duration::from_millis(150);

/// A running watch; dropping it stops the watch.
#[derive(Debug)]
pub struct ConfigWatch {
    _watcher: RecommendedWatcher,
}

/// Calls `on_change` on a thread of its own each time the file at `path`
/// changes, once the changes have settled for [`DEBOUNCE`].
///
/// # Errors
///
/// When the platform watcher cannot be created or the directory watched.
pub fn watch(
    path: &Path,
    mut on_change: impl FnMut() + Send + 'static,
) -> notify::Result<ConfigWatch> {
    let dir = path
        .parent()
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf);
    std::fs::create_dir_all(&dir).map_err(notify::Error::io)?;
    let name = path.file_name().map(std::ffi::OsStr::to_owned);
    let (sender, events) = std::sync::mpsc::channel::<()>();
    let mut watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
        let relevant = match &event {
            Ok(event) => {
                !event.kind.is_access()
                    && event
                        .paths
                        .iter()
                        .any(|changed| changed.file_name() == name.as_deref())
            }
            Err(_) => true,
        };
        if relevant {
            let _ = sender.send(());
        }
    })?;
    watcher.watch(&dir, RecursiveMode::NonRecursive)?;
    std::thread::Builder::new()
        .name("compass-config-watch".to_owned())
        .spawn(move || {
            while events.recv().is_ok() {
                while events.recv_timeout(DEBOUNCE).is_ok() {}
                on_change();
            }
        })
        .map_err(notify::Error::io)?;
    Ok(ConfigWatch { _watcher: watcher })
}

/// Every problem with `config` and the file it came from: `issues` as
/// [`Config::load_checked`] found them, plus a theme or preset that does not
/// exist.
#[must_use]
pub fn check(config: &Config, mut issues: Vec<ConfigIssue>) -> Vec<ConfigIssue> {
    let appearance = config.launcher().appearance();
    if let Some(theme) = appearance.theme_override()
        && compass_ui::theme::Theme::from_name(theme).is_none()
    {
        let mut names: Vec<String> = compass_ui::theme::Theme::ALL
            .iter()
            .map(|theme| theme.name().to_owned())
            .collect();
        names.extend(
            compass_ui::theme::load_user_themes(&compass_core::theme_file::default_search_dirs())
                .iter()
                .map(|theme| theme.name().to_owned()),
        );
        issues.push(ConfigIssue::unknown_name(
            "launcher.appearance.theme",
            "theme",
            theme,
            names.iter().map(String::as_str),
        ));
    }
    let preset = appearance.preset();
    if compass_ui::preset::Preset::from_name(preset).is_none() {
        issues.push(ConfigIssue::unknown_name(
            "launcher.appearance.preset",
            "layout preset",
            preset,
            compass_ui::preset::NAMES.iter().map(|(name, _)| *name),
        ));
    }
    let scheme = appearance.color_scheme();
    if !crate::appearance::ColorMode::is_known(scheme) {
        issues.push(ConfigIssue::unknown_name(
            "launcher.appearance.color_scheme",
            "color scheme",
            scheme,
            ["system", "light", "dark"],
        ));
    }
    let keybinding = config.launcher().keybinding();
    if !["default", "vim", "emacs"].contains(&keybinding) {
        issues.push(ConfigIssue::unknown_name(
            "launcher.keybinding",
            "navigation scheme",
            keybinding,
            ["default", "vim", "emacs"],
        ));
    }
    issues
}

/// Loads `compass.json` and [`check`]s it; `None` when it cannot be read or
/// is not JSON, which the caller reports as it already does.
#[must_use]
pub fn load_and_check() -> Option<(Config, Vec<ConfigIssue>)> {
    let (config, issues) = Config::load_with_issues().ok()?;
    let issues = check(&config, issues);
    Some((config, issues))
}

/// Logs each problem once, as a warning naming the file.
pub fn log_issues(issues: &[ConfigIssue]) {
    for issue in issues {
        tracing::warn!(key = %issue.key, "compass.json: {issue}");
    }
}

/// The engine's watch: on each change, log what is wrong and apply what can
/// be. Runs until the engine stops.
pub async fn run(state: Arc<RwLock<EngineState>>) {
    let Ok(path) = compass_core::config::default_config_path() else {
        return;
    };
    let (sender, mut changes) = tokio::sync::mpsc::unbounded_channel();
    let watch = watch(&path, move || {
        let _ = sender.send(());
    });
    let _watch = match watch {
        Ok(watch) => watch,
        Err(error) => {
            tracing::warn!(%error, path = %path.display(),
                "cannot watch compass.json; changes to it apply at the next start");
            return;
        }
    };
    while changes.recv().await.is_some() {
        let loaded = tokio::task::spawn_blocking(|| {
            let _ = compass_ui::theme::load_default_user_themes();
            match Config::load_with_issues() {
                Ok((config, issues)) => Ok((check(&config, issues), config)),
                Err(error) => Err(error),
            }
        })
        .await;
        match loaded {
            Ok(Ok((issues, config))) => {
                tracing::info!(path = %path.display(), "compass.json changed; applying it");
                log_issues(&issues);
                crate::serve::settings::apply_all(&state, &config).await;
            }
            Ok(Err(error)) => {
                tracing::warn!(%error, "compass.json changed but cannot be read; keeping the settings in use");
            }
            Err(_) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_change_to_the_file_is_reported_once_it_settles() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("compass.json");
        std::fs::write(&path, "{}").unwrap();
        let (sender, changes) = std::sync::mpsc::channel();
        let _watch = watch(&path, move || {
            let _ = sender.send(());
        })
        .unwrap();

        std::fs::write(dir.path().join("other.json"), "{}").unwrap();
        assert!(
            changes.recv_timeout(Duration::from_millis(500)).is_err(),
            "another file in the directory is not a change"
        );

        // An editor's save: a new file renamed over the old one.
        let staged = dir.path().join(".compass.json.tmp");
        std::fs::write(&staged, r#"{"launcher": {"max_results": 3}}"#).unwrap();
        std::fs::rename(&staged, &path).unwrap();
        std::fs::write(&path, r#"{"launcher": {"max_results": 4}}"#).unwrap();
        changes
            .recv_timeout(Duration::from_secs(5))
            .expect("the change is reported");
        assert!(
            changes.recv_timeout(DEBOUNCE * 3).is_err(),
            "a burst of writes is one change"
        );
    }

    #[test]
    fn a_theme_and_preset_that_do_not_exist_are_named_with_a_suggestion() {
        let config = Config::parse(
            r#"{"launcher": {"appearance": {"theme": "draculla", "preset": "rofii"}}}"#,
            Path::new("compass.json"),
        )
        .unwrap();
        let shown: Vec<String> = check(&config, Vec::new())
            .iter()
            .map(ToString::to_string)
            .collect();
        assert_eq!(
            shown,
            [
                "launcher.appearance.theme names the theme \"draculla\", which does not exist. Did you mean \"dracula\"?",
                "launcher.appearance.preset names the layout preset \"rofii\", which does not exist. Did you mean \"rofi\"?",
            ]
        );
    }
}
