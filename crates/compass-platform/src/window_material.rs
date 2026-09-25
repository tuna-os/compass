//! A material behind the launcher window: blur of what is underneath it
//! (`WindowMaterialBackend`, `services/window-material`).
//!
//! The launcher window says where its card is and how round its corners are;
//! how the platform is asked is the platform's (on Wayland,
//! `ext-background-effect-v1` on the toolkit's own surface, through
//! `compass-wayland-foreign`), handed to the window by the `compass` binary.
//! macOS vibrancy and Windows acrylic would be other implementations.

pub use raw_window_handle;

use raw_window_handle::{HasDisplayHandle, HasWindowHandle};

/// A toolkit window, as far as the platform needs one: its native handles.
///
/// Implemented for anything that lends both, so a toolkit's window type (or a
/// thin wrapper over a borrowed one) is one.
pub trait NativeWindow: HasWindowHandle + HasDisplayHandle {}

impl<T: HasWindowHandle + HasDisplayHandle + ?Sized> NativeWindow for T {}

/// Where the material goes, in the window's logical coordinates, and the
/// corner radius it is rounded to (`WindowMaterialBackend::Params`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MaterialRegion {
    /// Left edge.
    pub x: i32,
    /// Top edge.
    pub y: i32,
    /// Width.
    pub width: i32,
    /// Height.
    pub height: i32,
    /// The corner radius.
    pub radius: i32,
}

/// What asking for the material led to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MaterialOutcome {
    /// The window is blurred behind the region now (a new or changed region).
    Applied,
    /// It already was, with this region; nothing was sent.
    Unchanged,
    /// The material was taken away.
    Cleared,
    /// This desktop cannot do it (no protocol, no capability, not Wayland).
    Unsupported,
}

/// Puts the platform's material behind a launcher window.
pub trait WindowMaterial: Send {
    /// Blurs behind `window` in `region`, or takes the blur away (`None`).
    ///
    /// Called with the window's handles lent for the call, while the window
    /// is open; the material applies from the window's next frame.
    ///
    /// # Errors
    ///
    /// The platform's failure, as a sentence. A desktop that cannot blur is
    /// [`MaterialOutcome::Unsupported`], not an error.
    fn apply(
        &mut self,
        window: &dyn NativeWindow,
        region: Option<MaterialRegion>,
    ) -> Result<MaterialOutcome, String>;
}
