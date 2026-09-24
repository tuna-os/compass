//! Client bindings for Wayland protocols that no published crate carries yet.
//!
//! Generated at build time by `wayland-scanner` — the same generator
//! `wayland-protocols` and `wayland-protocols-wlr` use — from XML checked in
//! under `protocols/`. Nothing here is hand-written protocol code; the crate
//! exists only because the generated interface tables contain `unsafe`, which
//! the rest of the workspace forbids outright.
//!
//! - [`xx_hotkey_v1`]: client-managed global hotkeys, the protocol the C++
//!   engine's `XxHotkeyGlobalShortcutBackend` speaks (upstream vicinae #1936;
//!   the XML is a copy of `src/wayland-protocols/xx-hotkey-v1.xml`). No
//!   `xdg-desktop-portal-wlr` release has a GlobalShortcuts backend, so on a
//!   wlroots compositor this is the only way an unprivileged client can ask
//!   for a hotkey.

/// `xx-hotkey-v1`, experimental.
#[allow(unsafe_code, missing_docs, clippy::all)]
pub mod xx_hotkey_v1 {
    use wayland_client;
    use wayland_client::protocol::*;

    /// The raw interface descriptors.
    pub mod __interfaces {
        use wayland_client::protocol::__interfaces::*;
        wayland_scanner::generate_interfaces!("protocols/xx-hotkey-v1.xml");
    }
    use self::__interfaces::*;

    wayland_scanner::generate_client_code!("protocols/xx-hotkey-v1.xml");
}
