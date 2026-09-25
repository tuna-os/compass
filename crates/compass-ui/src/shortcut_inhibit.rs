//! The compositor's shortcuts while a shortcut recorder records
//! (`ShortcutInhibitManager`, `ShortcutInhibitorAttached`).
//!
//! Under the layer-shell presentation the launcher makes its own Wayland
//! connection and hands it to `iced_layershell`, so the inhibitor
//! ([`compass_wayland::keyboard_inhibit`]) sees the launcher's surfaces take
//! and lose the keyboard on the same connection. The toolkit reads the
//! socket; the inhibitor takes what was read for it on the UI thread, at each
//! update. Under the `xdg_toplevel` presentation winit
//! makes its connection itself and offers no way to share it, so there is
//! nothing to inhibit with and [`set_recording`] does nothing (`PARITY.md`,
//! "The gaps pass, wlroots paste and inhibit").

#[cfg(target_os = "linux")]
static INHIBIT: std::sync::OnceLock<std::sync::Mutex<compass_wayland::ShortcutInhibit>> =
    std::sync::OnceLock::new();

/// Binds the inhibitor on the launcher's connection, before the toolkit
/// starts reading it. A compositor without the protocol is logged and left
/// alone.
#[cfg(target_os = "linux")]
pub(crate) fn install(connection: &wayland_client::Connection) {
    match compass_wayland::ShortcutInhibit::bind(connection) {
        Ok(inhibit) => {
            let _ = INHIBIT.set(std::sync::Mutex::new(inhibit));
        }
        Err(error) => tracing::info!(%error, "the shortcut recorder cannot inhibit shortcuts"),
    }
}

#[cfg(target_os = "linux")]
fn with(apply: impl FnOnce(&mut compass_wayland::ShortcutInhibit)) {
    if let Some(inhibit) = INHIBIT.get() {
        apply(
            &mut inhibit
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        );
    }
}

/// Asks for the compositor's shortcuts to reach the launcher while
/// `recording`, and gives them back after.
pub(crate) fn set_recording(recording: bool) {
    #[cfg(target_os = "linux")]
    with(|inhibit| inhibit.set_wanted(recording));
    #[cfg(not(target_os = "linux"))]
    let _ = recording;
}

/// Takes the keyboard's comings and goings read since the last update, so
/// the inhibitor follows the launcher surface that holds the keyboard.
pub(crate) fn follow_focus() {
    #[cfg(target_os = "linux")]
    with(|inhibit| {
        if let Err(error) = inhibit.dispatch_pending() {
            tracing::info!(%error, "the shortcut inhibitor's connection");
        }
    });
}
