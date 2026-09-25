//! `compass-ui` — The Iced-based launcher UI wired to real search results.
//!
//! This crate provides the main launcher application using Iced 0.14 with
//! the Wayland backend. It connects the UI to the real application index
//! and handles the launcher lifecycle.

#![deny(missing_docs)]

/// The launcher window's Wayland `app_id`: the basename of the desktop file
/// the Flatpak installs, so the Shell ties the window to that entry. The
/// window switcher also recognises its own window by it, because inside the
/// Flatpak's pid namespace `std::process::id()` is not the pid the Shell sees.
pub const APP_ID: &str = "com.vicinae.Vicinae";

pub mod action_panel;
pub mod app;
pub mod appearance;
pub mod apps_page;
pub mod backend;
pub mod calculator_page;
pub mod clipboard_page;
pub mod design;
pub mod developer_page;
pub mod dmenu_page;
pub mod emoji_page;
pub mod extension_fields;
pub mod extension_page;
pub mod fallbacks_page;
pub mod file_preview;
pub mod files_page;
pub mod fonts_page;
pub mod grants_page;
pub mod hud;
pub mod icons;
pub mod media_page;
pub mod message;
pub mod onboarding_page;
pub mod open_with_page;
pub mod preferences_page;
pub mod preset;
pub mod programs_page;
pub mod remote_image;
pub mod resident;
pub mod root_list;
pub mod script_page;
mod scroll;
pub mod settings;
pub mod settings_page;
mod shortcut_inhibit;
pub mod shortcut_recorder;
pub mod shortcuts_page;
pub mod snippets_page;
pub mod store_page;
pub mod surface;
pub mod theme;
pub mod themes_page;
pub mod tray_page;
pub mod typography;
pub mod vicinae_pages;
pub mod view_memory;
pub mod windows_page;
pub mod workspaces_page;

pub use app::{AppFlags, ClockSettings, Dismissal, LauncherApp, next_selection};
pub use appearance::{AppearanceLink, AppearanceSender};
pub use typography::{TypographyLink, TypographySender};

/// Opens the launcher window and runs until it closes.
///
/// Takes over the calling thread: Iced's event loop owns it, and on Wayland it
/// must be the process's main thread. That constraint is the whole reason this
/// is a separate entry point rather than something `vicinae serve` calls — see
/// ADR-0011.
///
/// # Errors
///
/// Returns Iced's error when the window cannot be created, which on a machine
/// with no compositor is the normal outcome rather than a bug.
pub fn run(flags: AppFlags) -> iced::Result {
    let window = flags.window_config.clone();
    iced::application(
        move || LauncherApp::new(flags.clone()),
        LauncherApp::update,
        LauncherApp::view,
    )
    .theme(LauncherApp::theme)
    .subscription(LauncherApp::subscription)
    .window(window)
    .run()
}
pub use message::{Direction, Message};
pub use resident::{EngineLink, UiCommand, UiOutcome};

/// Runs the launcher as a resident process, driven by an engine.
///
/// Unlike [`run`], this does not end when something is launched: it hides and
/// waits to be summoned again ([ADR-0015](../../docs/rust-engine/adr/0015-the-launcher-window-is-resident.md)).
/// A window is opened at boot, so starting it by hand still puts a launcher on
/// screen rather than an invisible process.
///
/// Takes over the calling thread for the same reason [`run`] does.
///
/// # Errors
///
/// Returns Iced's error when the event loop cannot start, which on a machine
/// with no compositor is the normal outcome rather than a bug.
pub fn run_resident(flags: AppFlags) -> iced::Result {
    // `iced::daemon`, NOT `iced::application`, AND THE DIFFERENCE IS THE WHOLE
    // FEATURE.
    //
    // `iced_winit` ends the process when the last window closes -- but that
    // branch is guarded by `!is_daemon`, and `is_daemon` is simply
    // `window_settings.is_none()`. `daemon`'s `Program::window()` returns
    // `None`; `application`'s returns `Some(..)`.
    //
    // So under `application`, hiding the launcher would kill it. Every test in
    // this workspace would still pass, because none of them run a real event
    // loop -- the state machine would report `Hidden` quite correctly to an
    // engine whose window had just exited. Verified by reading the shipped
    // iced 0.14 source rather than inferred from the names.
    //
    // Named functions rather than closures: `iced::daemon`'s view takes a
    // higher-ranked lifetime, and a closure's inferred signature is not general
    // enough to satisfy it.
    fn view(app: &LauncherApp, window: iced::window::Id) -> iced::Element<'_, Message> {
        app.view_for(window)
    }
    fn title(app: &LauncherApp, _window: iced::window::Id) -> String {
        app.title()
    }
    fn theme(app: &LauncherApp, _window: iced::window::Id) -> iced::Theme {
        app.theme()
    }

    iced::daemon(
        move || LauncherApp::boot(flags.clone()),
        LauncherApp::update,
        view,
    )
    .title(title)
    .theme(theme)
    .subscription(LauncherApp::subscription)
    .run()
}

/// [`run_resident`], presenting the launcher as a `wlr-layer-shell`
/// surface through `iced_layershell` instead of an `xdg_toplevel`.
///
/// For the wlroots family only; the binary decides, from the compositor's
/// registry and never on GNOME (`compass_wayland::select_surface`). The app
/// is the same `LauncherApp` and behaves the same — resident, hidden by
/// closing the surface, reopened on `Show` — because the only difference is
/// how the window is asked for (`surface::open`).
///
/// # Errors
///
/// `iced_layershell`'s error when the compositor has no layer shell or the
/// event loop cannot start.
#[cfg(target_os = "linux")]
pub fn run_resident_layer_shell(flags: AppFlags) -> Result<(), iced_layershell::Error> {
    use iced_layershell::settings::{LayerShellSettings, Settings, StartMode};

    fn view(app: &LauncherApp, window: iced::window::Id) -> iced::Element<'_, Message> {
        app.view_for(window)
    }
    fn title(app: &LauncherApp, _window: iced::window::Id) -> Option<String> {
        Some(app.title())
    }
    fn theme(app: &LauncherApp, _window: iced::window::Id) -> iced::Theme {
        app.theme()
    }

    surface::set_presentation(surface::Presentation::LayerShell);
    // The launcher's own connection, shared with `iced_layershell`, so the
    // shortcut inhibitor is told which of its surfaces holds the keyboard.
    // Without one `iced_layershell` connects itself and reports why it could
    // not.
    let with_connection = match wayland_client::Connection::connect_to_env() {
        Ok(connection) => {
            shortcut_inhibit::install(&connection);
            Some(connection.into())
        }
        Err(error) => {
            tracing::info!(%error, "no Wayland connection to share with the layer shell");
            None
        }
    };
    iced_layershell::build_pattern::daemon(
        move || LauncherApp::boot(flags.clone()),
        surface::layer::NAMESPACE,
        LauncherApp::update,
        view,
    )
    .title(title)
    .theme(theme)
    .subscription(LauncherApp::subscription)
    .settings(Settings {
        id: Some(APP_ID.to_owned()),
        // No surface at start: the first one comes from `LauncherApp::boot`
        // through `surface::open`, exactly as the first window does under
        // `iced::daemon`, so `start_hidden` means the same thing on both.
        layer_settings: LayerShellSettings {
            start_mode: StartMode::Background,
            ..LayerShellSettings::default()
        },
        with_connection,
        ..Settings::default()
    })
    .run()
}
