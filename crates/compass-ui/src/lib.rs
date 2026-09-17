//! `compass-ui` — The Iced-based launcher UI wired to real search results.
//!
//! This crate provides the main launcher application using Iced 0.14 with
//! the Wayland backend. It connects the UI to the real application index
//! and handles the launcher lifecycle.

#![deny(missing_docs)]

pub mod app;
pub mod message;
pub mod resident;

pub use app::{AppFlags, Dismissal, LauncherApp, next_selection};

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
    // Named functions rather than closures: `iced::daemon`'s view takes a
    // higher-ranked lifetime, and a closure's inferred signature is not general
    // enough to satisfy it.
    fn view(app: &LauncherApp, _window: iced::window::Id) -> iced::Element<'_, Message> {
        app.view()
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
