//! The compositor's shortcuts while a shortcut recorder records
//! (`ShortcutInhibitManager`, `ShortcutInhibitorAttached`).
//!
//! Under the layer-shell presentation the `vicinae` binary makes the
//! launcher's Wayland connection, hands it to `iced_layershell`, and binds an
//! inhibitor on it ([`compass_platform::ShortcutInhibitor`]), so the inhibitor
//! sees the launcher's surfaces take and lose the keyboard on the same
//! connection. The toolkit reads the socket; the inhibitor takes what was read
//! for it on the UI thread, at each update. Under the `xdg_toplevel`
//! presentation winit makes its connection itself and offers no way to share
//! it, so none is installed and [`set_recording`] does nothing (`PARITY.md`,
//! "The gaps pass, wlroots paste and inhibit").

use compass_platform::ShortcutInhibitor;

static INHIBIT: std::sync::OnceLock<std::sync::Mutex<Box<dyn ShortcutInhibitor>>> =
    std::sync::OnceLock::new();

/// Keeps the inhibitor the binary bound on the launcher's connection.
pub(crate) fn install(inhibitor: Box<dyn ShortcutInhibitor>) {
    let _ = INHIBIT.set(std::sync::Mutex::new(inhibitor));
}

fn with(apply: impl FnOnce(&mut dyn ShortcutInhibitor)) {
    if let Some(inhibit) = INHIBIT.get() {
        apply(
            inhibit
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .as_mut(),
        );
    }
}

/// Asks for the compositor's shortcuts to reach the launcher while
/// `recording`, and gives them back after.
pub(crate) fn set_recording(recording: bool) {
    with(|inhibit| inhibit.set_wanted(recording));
}

/// Takes the keyboard's comings and goings read since the last update, so
/// the inhibitor follows the launcher surface that holds the keyboard.
pub(crate) fn follow_focus() {
    with(|inhibit| {
        if let Err(error) = inhibit.dispatch_pending() {
            tracing::info!(%error, "the shortcut inhibitor's connection");
        }
    });
}
