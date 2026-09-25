//! Blur behind the launcher window on Wayland (`ExtBackgroundEffectV1Manager`,
//! `WindowMaterialManager::createBackend`).
//!
//! The window lends its native handles for each request; they are bridged
//! into a connection over winit's own display and a proxy of the launcher's
//! surface (`compass-wayland-foreign`, the `unsafe` exception, ADR-0019), and
//! `compass_wayland::material::BackgroundEffects`, bound once on that
//! connection, sets the rounded blur region. A compositor without
//! `ext_background_effect_manager_v1` (Sway, Mutter) is remembered and never
//! asked again; one with the manager but not the blur capability is asked
//! again at each change, since the capability can arrive later.

use compass_platform::{MaterialOutcome, MaterialRegion, NativeWindow, WindowMaterial};
use compass_wayland::material::{Applied, BackgroundEffects, MaterialError, Params, Rect};
use compass_wayland_foreign::{BridgeError, bridge};

/// The launcher's material: `ext-background-effect-v1`, bound on first use.
#[derive(Default)]
pub struct LauncherMaterial {
    bound: Option<(wayland_client::Connection, BackgroundEffects)>,
    /// The desktop cannot do it at all: not Wayland, or no manager.
    unsupported: bool,
}

impl std::fmt::Debug for LauncherMaterial {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LauncherMaterial")
            .field("bound", &self.bound.is_some())
            .field("unsupported", &self.unsupported)
            .finish()
    }
}

/// What the `xdg_toplevel` launcher is given.
#[must_use]
pub fn for_launcher() -> Box<dyn WindowMaterial> {
    Box::new(LauncherMaterial::default())
}

/// The protocol client's parameters for a region.
#[must_use]
pub fn params(region: MaterialRegion) -> Params {
    Params {
        radius: region.radius,
        region: Rect {
            x: region.x,
            y: region.y,
            width: region.width,
            height: region.height,
        },
    }
}

/// What the protocol client did, as the window is told it.
///
/// # Errors
///
/// The surface was destroyed under the request, which the window's lending
/// its handles for the call should make impossible.
pub fn outcome(applied: Applied) -> Result<MaterialOutcome, String> {
    match applied {
        Applied::Created | Applied::Updated => Ok(MaterialOutcome::Applied),
        Applied::Unchanged => Ok(MaterialOutcome::Unchanged),
        Applied::Unsupported => Ok(MaterialOutcome::Unsupported),
        Applied::SurfaceGone => Err("the launcher's surface is gone".to_owned()),
    }
}

impl WindowMaterial for LauncherMaterial {
    fn apply(
        &mut self,
        window: &dyn NativeWindow,
        region: Option<MaterialRegion>,
    ) -> Result<MaterialOutcome, String> {
        if self.unsupported {
            return Ok(MaterialOutcome::Unsupported);
        }
        let bridged = match bridge(window) {
            Ok(bridged) => bridged,
            Err(BridgeError::NotWayland | BridgeError::NoLibrary) => {
                self.unsupported = true;
                return Ok(MaterialOutcome::Unsupported);
            }
            Err(error) => return Err(error.to_string()),
        };
        // One manager per display; a window on another display (none today)
        // would get its own.
        if self
            .bound
            .as_ref()
            .is_none_or(|(connection, _)| connection.backend() != bridged.connection.backend())
        {
            match BackgroundEffects::bind(&bridged.connection) {
                Ok(effects) => self.bound = Some((bridged.connection.clone(), effects)),
                Err(MaterialError::Unsupported) => {
                    self.unsupported = true;
                    return Ok(MaterialOutcome::Unsupported);
                }
                Err(error) => return Err(error.to_string()),
            }
        }
        let Some((_, effects)) = self.bound.as_mut() else {
            return Ok(MaterialOutcome::Unsupported);
        };
        effects
            .dispatch_pending()
            .map_err(|error| error.to_string())?;
        let result = match region {
            Some(region) => outcome(effects.apply(&bridged.surface, params(region))),
            None if effects.clear(&bridged.surface) => Ok(MaterialOutcome::Cleared),
            None => Ok(MaterialOutcome::Unchanged),
        };
        effects.flush().map_err(|error| error.to_string())?;
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use compass_platform::raw_window_handle::{
        DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, WindowHandle,
    };

    /// A window on another platform: a display handle that is not Wayland.
    struct Elsewhere;

    impl HasDisplayHandle for Elsewhere {
        fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
            Ok(DisplayHandle::windows())
        }
    }

    impl HasWindowHandle for Elsewhere {
        fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
            Err(HandleError::NotSupported)
        }
    }

    #[test]
    fn the_region_is_passed_through_as_the_cpp_params() {
        let region = MaterialRegion {
            x: 24,
            y: 24,
            width: 768,
            height: 312,
            radius: 16,
        };
        assert_eq!(
            params(region),
            Params {
                radius: 16,
                region: Rect {
                    x: 24,
                    y: 24,
                    width: 768,
                    height: 312,
                },
            }
        );
    }

    #[test]
    fn what_the_client_did_is_what_the_window_is_told() {
        assert_eq!(outcome(Applied::Created), Ok(MaterialOutcome::Applied));
        assert_eq!(outcome(Applied::Updated), Ok(MaterialOutcome::Applied));
        assert_eq!(outcome(Applied::Unchanged), Ok(MaterialOutcome::Unchanged));
        assert_eq!(
            outcome(Applied::Unsupported),
            Ok(MaterialOutcome::Unsupported)
        );
        assert!(outcome(Applied::SurfaceGone).is_err());
    }

    #[test]
    fn a_window_that_is_not_wayland_is_unsupported_and_not_asked_again() {
        let mut material = LauncherMaterial::default();
        let region = Some(MaterialRegion::default());
        assert_eq!(
            material.apply(&Elsewhere, region),
            Ok(MaterialOutcome::Unsupported)
        );
        assert!(material.unsupported);
        assert_eq!(
            material.apply(&Elsewhere, None),
            Ok(MaterialOutcome::Unsupported)
        );
    }
}
