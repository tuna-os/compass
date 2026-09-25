//! Inhibiting the compositor's shortcuts while a shortcut recorder records
//! (`ShortcutInhibitManager`).
//!
//! The launcher window asks; how the compositor is asked is the platform's
//! (on Wayland, `compass_wayland::ShortcutInhibit` over
//! keyboard-shortcuts-inhibit), handed to the window by the `vicinae` binary.

/// What the launcher window asks of the compositor while a shortcut recorder
/// records: its shortcuts, so a combination it would act on can be recorded.
pub trait ShortcutInhibitor: Send {
    /// Asks for the compositor's shortcuts to reach the launcher (`true`), or
    /// gives them back (`false`).
    fn set_wanted(&mut self, wanted: bool);

    /// Takes what was read for the inhibitor since the last call, so it
    /// follows the surface that holds the keyboard.
    ///
    /// # Errors
    ///
    /// The connection's error, as a sentence.
    fn dispatch_pending(&mut self) -> Result<(), String>;
}
