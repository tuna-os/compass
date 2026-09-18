//! The engine's application state: what can be launched, what the user launches, and how they
//! configured it.
//!
//! Three pieces, deliberately independent of any front end:
//!
//! * [`apps`] — scan the XDG application directories and turn `.desktop` files into
//!   [`AppItem`]s that [`compass_search`] can rank.
//! * [`frecency`] — remember what was launched and when, so ranking can prefer it.
//! * [`config`] — the `vicinae.json` user configuration.
//!
//! ```
//! use compass_core::{AppIndex, Config};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! # let dir = tempfile::tempdir()?;
//! # std::fs::write(
//! #     dir.path().join("firefox.desktop"),
//! #     "[Desktop Entry]\nType=Application\nName=Firefox\nExec=firefox %u\n",
//! # )?;
//! let index = AppIndex::builder().dir(dir.path()).build();
//! let hits = index.search("fir");
//! assert_eq!(hits[0].item.name(), "Firefox");
//!
//! let config = Config::default();
//! assert_eq!(config.launcher().max_results(), 50);
//! # Ok(())
//! # }
//! ```
//!
//! Nothing here reads the environment except [`xdg_dirs`] and the `from_environment` /
//! `default_*_path` constructors that call it. Every other entry point takes explicit paths, so
//! tests never touch the invoking user's `$HOME`.

#![deny(missing_docs)]

pub mod alert;
pub mod app_service;
pub mod app_windows;
pub mod apps;
pub mod asset_resolver;
pub mod audio_control;
pub mod boilerplate;
pub mod browse_apps;
pub mod builtin_icon;
pub mod config;
pub mod create_extension;
pub mod default_app;
pub mod entry_filter;
pub mod extension_store;
pub mod fetch_queue;
pub mod file_category;
pub mod file_chooser;
pub mod font_browser;
pub mod frecency;
pub mod glyph;
pub mod glyph_service;
pub mod keybinding;
pub mod list_navigation;
pub mod manifest;
pub mod news;
pub mod paste;
pub mod power_commands;
pub mod query_policy;
pub mod rank;
pub mod raycast_store;
pub mod root_items;
pub mod script_command;
pub mod selection;
pub mod semver;
pub mod shortcut;
pub mod shortcut_form;
pub mod shortcut_store;
pub mod slug;
pub mod snippet;
pub mod snippet_form;
pub mod system_run;
pub mod telemetry;
pub mod theme_picker;
pub mod toast;
pub mod update;
pub mod xdg_dirs;

pub use apps::{AppIndex, AppIndexBuilder, AppItem, SkipReason, SkippedEntry};
pub use config::{Config, ConfigError, ExtensionsConfig, LauncherConfig};
pub use frecency::{
    Clock, FrecencyError, FrecencyRecord, FrecencyStore, JsonFrecencyStore, ManualClock,
    SystemClock,
};
pub use rank::{Ranked, rank_with_frecency};

use std::path::Path;

/// Writes `data` to `path` via a temporary file in the same directory, then renames it into
/// place. Parent directories are created.
///
/// Rename is atomic within a filesystem, so a reader either sees the old file or the new one; a
/// crash mid-write cannot leave a truncated config or a truncated launch history behind.
fn atomic_write(path: &Path, data: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)?;
    }

    let file_name = path
        .file_name()
        .ok_or_else(|| std::io::Error::other(format!("{} is not a file path", path.display())))?;
    let mut tmp = path.to_path_buf();
    tmp.set_file_name(format!(
        "{}.tmp-{}",
        file_name.to_string_lossy(),
        std::process::id()
    ));

    std::fs::write(&tmp, data)?;
    match std::fs::rename(&tmp, path) {
        Ok(()) => Ok(()),
        Err(err) => {
            let _ = std::fs::remove_file(&tmp);
            Err(err)
        }
    }
}
