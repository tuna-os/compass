//! Blur behind the launcher's card (`WindowMaterialAttached`,
//! `services/window-material`).
//!
//! The binary hands the window a [`compass_platform::WindowMaterial`] (on
//! Wayland, `ext-background-effect-v1` on the toolkit's own surface); the
//! window asks for the card's rounded rectangle whenever the card is shown,
//! resized, or its translucency or corner radius change, as the C++ re-applies
//! on `widthChanged`/`heightChanged` and on a new surface. The request runs
//! inside `iced::window::run`, where the toolkit lends the window's native
//! handles for the call.
//!
//! Under the `xdg_toplevel` presentation only: `iced_layershell` does not run
//! `window::run` (its runtime drops the action), so a layer-surface launcher
//! has no handles to lend and is not blurred (`PARITY.md`, "The
//! window-material pass").

use std::sync::{Mutex, OnceLock, PoisonError};

use compass_platform::raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, WindowHandle,
};
use compass_platform::{MaterialOutcome, MaterialRegion, WindowMaterial};
use iced::Task;
use iced::window;

use crate::message::Message;

static MATERIAL: OnceLock<Mutex<Box<dyn WindowMaterial>>> = OnceLock::new();

/// Keeps the material the binary chose.
pub(crate) fn install(material: Box<dyn WindowMaterial>) {
    let _ = MATERIAL.set(Mutex::new(material));
}

/// Whether there is one to ask.
pub(crate) fn installed() -> bool {
    MATERIAL.get().is_some()
}

/// The card's region: where `view` puts it (inside the shadow padding), its
/// measured size, and its corner radius. `None` when the card is opaque,
/// which takes the blur away, as `WindowMaterial.enabled: blurEnabled` does.
pub(crate) fn region(
    tint: bool,
    card: iced::Size,
    padding: u16,
    radius: u16,
) -> Option<MaterialRegion> {
    if !tint || card.width <= 0.0 || card.height <= 0.0 {
        return None;
    }
    let edge = i32::from(padding);
    #[allow(clippy::cast_possible_truncation)]
    Some(MaterialRegion {
        x: edge,
        y: edge,
        width: card.width.round() as i32,
        height: card.height.round() as i32,
        radius: i32::from(radius),
    })
}

/// Asks for `region` behind window `id`, with the window's handles lent for
/// the call. Nothing when no material was installed.
pub(crate) fn apply(id: window::Id, region: Option<MaterialRegion>) -> Task<Message> {
    if !installed() {
        return Task::none();
    }
    window::run(id, move |window| {
        let Some(material) = MATERIAL.get() else {
            return;
        };
        let mut material = material.lock().unwrap_or_else(PoisonError::into_inner);
        match material.apply(&Lent(window), region) {
            Ok(MaterialOutcome::Unsupported) => {
                tracing::debug!("this desktop does not blur behind windows");
            }
            Ok(outcome) => tracing::debug!(?outcome, ?region, "the launcher's material"),
            Err(error) => tracing::info!(%error, "the launcher's material"),
        }
    })
    .discard()
}

/// The toolkit's window, lent for one call.
struct Lent<'a>(&'a dyn window::Window);

impl HasWindowHandle for Lent<'_> {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        self.0.window_handle()
    }
}

impl HasDisplayHandle for Lent<'_> {
    fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
        self.0.display_handle()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_opaque_card_asks_for_no_blur() {
        assert_eq!(region(false, iced::Size::new(768.0, 560.0), 24, 16), None);
    }

    #[test]
    fn a_translucent_card_is_blurred_where_it_is_drawn() {
        assert_eq!(
            region(true, iced::Size::new(768.0, 312.4), 24, 16),
            Some(MaterialRegion {
                x: 24,
                y: 24,
                width: 768,
                height: 312,
                radius: 16,
            })
        );
    }

    #[test]
    fn a_card_not_laid_out_yet_asks_for_nothing() {
        assert_eq!(region(true, iced::Size::ZERO, 24, 16), None);
    }

    #[test]
    fn without_a_material_nothing_is_asked() {
        // Nothing in the test process installs one.
        assert!(!installed());
        assert_eq!(apply(window::Id::unique(), None).units(), 0);
    }
}
