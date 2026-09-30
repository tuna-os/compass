//! `ext-background-effect-v1`: the compositor blurs what is behind a
//! translucent surface, in a region the client sets.
//!
//! Ports `ExtBackgroundEffectV1Manager` (`services/window-material`) and
//! `QtWaylandUtils::createRoundedRegion`. The manager is bound on a
//! connection, its one-shot `capabilities` event is read with a roundtrip
//! before support is reported (as the C++ does), and each surface gets one
//! effect object whose blur region is set again only when its parameters
//! change. The region is the window's rectangle with its corners cut into
//! quarter circles, one pixel row at a time, so the blur does not show past
//! a rounded card.
//!
//! The effect is set on a `wl_surface` of the *same* display. The
//! launcher's surface belongs to the toolkit's connection (winit or
//! `iced_layershell`), which the toolkits expose only as raw pointers; this
//! module takes the surface as a safe proxy, and `compass-wayland-foreign`
//! (the workspace's `unsafe` exception for it, ADR-0019) makes that proxy and
//! a connection over the toolkit's display for the `compass` binary
//! (`PARITY.md`, "The window-material pass").
//!
//! The toolkit destroys the launcher's surface each time it hides. Setting a
//! blur region on an effect whose surface is gone is a protocol error, which
//! on a shared display would end the toolkit's connection too, so effects of
//! destroyed surfaces are dropped before anything is sent (`apply`).

use std::collections::HashMap;

use wayland_client::backend::ObjectId;
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::{wl_compositor, wl_region, wl_registry, wl_surface};
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum};
use wayland_protocols::ext::background_effect::v1::client::{
    ext_background_effect_manager_v1::{self, ExtBackgroundEffectManagerV1},
    ext_background_effect_surface_v1::{self, ExtBackgroundEffectSurfaceV1},
};

/// A rectangle in surface coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Rect {
    /// Left edge.
    pub x: i32,
    /// Top edge.
    pub y: i32,
    /// Width.
    pub width: i32,
    /// Height.
    pub height: i32,
}

/// One step of building a `wl_region`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegionOp {
    /// `wl_region.add`.
    Add(Rect),
    /// `wl_region.subtract`.
    Subtract(Rect),
}

/// `createRoundedRegion`: `region`, then for each of the `radius` rows at
/// the top and bottom, the part of each corner outside a circle of `radius`
/// taken away.
#[must_use]
pub fn rounded_region(region: Rect, radius: i32) -> Vec<RegionOp> {
    let Rect {
        x,
        y,
        width,
        height,
    } = region;
    let mut ops = vec![RegionOp::Add(region)];
    for row in 0..radius.max(0) {
        let r = f64::from(radius);
        let inset = f64::from(radius - row);
        let covered = (r * r - inset * inset).sqrt();
        #[allow(clippy::cast_possible_truncation)]
        let cut = radius - covered as i32;
        if cut <= 0 {
            continue;
        }
        let strip = |left: i32, top: i32| {
            RegionOp::Subtract(Rect {
                x: left,
                y: top,
                width: cut,
                height: 1,
            })
        };
        ops.push(strip(x, y + row));
        ops.push(strip(x + width - cut, y + row));
        ops.push(strip(x, y + height - 1 - row));
        ops.push(strip(x + width - cut, y + height - 1 - row));
    }
    ops
}

/// What a surface's effect is set to (`WindowMaterialBackend::Params`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Params {
    /// The corner radius the region is rounded to.
    pub radius: i32,
    /// The region blurred.
    pub region: Rect,
}

/// What applying led to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Applied {
    /// The surface got an effect.
    Created,
    /// Its region changed.
    Updated,
    /// Nothing changed, so nothing was sent.
    Unchanged,
    /// The compositor cannot blur.
    Unsupported,
    /// The surface was destroyed, so nothing was sent.
    SurfaceGone,
}

/// Why the manager could not be used.
#[derive(Debug, thiserror::Error)]
pub enum MaterialError {
    /// The registry could not be read.
    #[error("the compositor's registry: {0}")]
    Connection(String),
    /// The compositor advertises no `ext_background_effect_manager_v1`.
    #[error("the compositor has no ext_background_effect_manager_v1")]
    Unsupported,
}

#[derive(Default)]
struct State {
    capabilities: u32,
}

/// One surface's effect, and what it was set to.
struct Effect {
    surface: wl_surface::WlSurface,
    effect: ExtBackgroundEffectSurfaceV1,
    params: Params,
}

/// The background effect of every surface this client blurs.
pub struct BackgroundEffects {
    queue: EventQueue<State>,
    state: State,
    manager: ExtBackgroundEffectManagerV1,
    compositor: wl_compositor::WlCompositor,
    effects: HashMap<ObjectId, Effect>,
}

impl BackgroundEffects {
    /// Binds the manager on `connection` and reads its capabilities.
    ///
    /// # Errors
    ///
    /// [`MaterialError::Unsupported`] where the compositor has no manager
    /// (or no `wl_compositor` to make regions with), and
    /// [`MaterialError::Connection`] when the registry cannot be read.
    pub fn bind(connection: &Connection) -> Result<Self, MaterialError> {
        let (globals, mut queue) = registry_queue_init::<State>(connection)
            .map_err(|err| MaterialError::Connection(err.to_string()))?;
        let qh = queue.handle();
        let manager = globals
            .bind::<ExtBackgroundEffectManagerV1, _, _>(&qh, 1..=1, ())
            .map_err(|_| MaterialError::Unsupported)?;
        let compositor = globals
            .bind::<wl_compositor::WlCompositor, _, _>(&qh, 1..=6, ())
            .map_err(|_| MaterialError::Unsupported)?;
        let mut state = State::default();
        // The capabilities are sent once, at bind: read them before saying
        // whether blur is supported, as the C++'s roundtrip does.
        queue
            .roundtrip(&mut state)
            .map_err(|err| MaterialError::Connection(err.to_string()))?;
        Ok(Self {
            queue,
            state,
            manager,
            compositor,
            effects: HashMap::new(),
        })
    }

    /// Whether the compositor blurs now (`capability_blur`).
    #[must_use]
    pub fn supports_blur(&self) -> bool {
        supports_blur(self.state.capabilities)
    }

    /// Reads capability changes that have arrived.
    ///
    /// # Errors
    ///
    /// When the connection failed.
    pub fn dispatch_pending(&mut self) -> Result<(), MaterialError> {
        self.queue
            .dispatch_pending(&mut self.state)
            .map(drop)
            .map_err(|err| MaterialError::Connection(err.to_string()))
    }

    /// Blurs behind `surface` in `params`' rounded region; the region takes
    /// effect at the surface's next commit, which the toolkit makes.
    pub fn apply(&mut self, surface: &wl_surface::WlSurface, params: Params) -> Applied {
        self.forget_destroyed();
        if !surface.is_alive() {
            return Applied::SurfaceGone;
        }
        if !self.supports_blur() {
            return Applied::Unsupported;
        }
        let qh = self.queue.handle();
        let applied = match self.effects.get_mut(&surface.id()) {
            Some(current) if current.params == params => return Applied::Unchanged,
            Some(current) => {
                current.params = params;
                Applied::Updated
            }
            None => {
                let effect = self.manager.get_background_effect(surface, &qh, ());
                self.effects.insert(
                    surface.id(),
                    Effect {
                        surface: surface.clone(),
                        effect,
                        params,
                    },
                );
                Applied::Created
            }
        };
        if let Some(Effect { effect, .. }) = self.effects.get(&surface.id()) {
            let region = self.compositor.create_region(&qh, ());
            for op in rounded_region(params.region, params.radius) {
                match op {
                    RegionOp::Add(r) => region.add(r.x, r.y, r.width, r.height),
                    RegionOp::Subtract(r) => region.subtract(r.x, r.y, r.width, r.height),
                }
            }
            effect.set_blur_region(Some(&region));
            region.destroy();
        }
        applied
    }

    /// Takes the effect away from `surface`; `false` when it had none.
    pub fn clear(&mut self, surface: &wl_surface::WlSurface) -> bool {
        self.forget_destroyed();
        match self.effects.remove(&surface.id()) {
            Some(Effect { effect, .. }) => {
                effect.destroy();
                true
            }
            None => false,
        }
    }

    /// Drops the effects of surfaces the toolkit has destroyed. Their effect
    /// objects are inert; destroying one is allowed, setting its region is
    /// not.
    fn forget_destroyed(&mut self) {
        self.effects.retain(|_, entry| {
            let alive = entry.surface.is_alive();
            if !alive {
                entry.effect.destroy();
            }
            alive
        });
    }

    /// Sends what was asked, rather than waiting for the toolkit to flush.
    ///
    /// # Errors
    ///
    /// When the connection failed.
    pub fn flush(&self) -> Result<(), MaterialError> {
        self.queue
            .flush()
            .map_err(|err| MaterialError::Connection(err.to_string()))
    }

    /// How many surfaces have an effect.
    #[must_use]
    pub fn effect_count(&self) -> usize {
        self.effects.len()
    }
}

/// Whether a `capabilities` bitfield includes blur.
#[must_use]
pub fn supports_blur(capabilities: u32) -> bool {
    capabilities & ext_background_effect_manager_v1::Capability::Blur.bits() != 0
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for State {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ExtBackgroundEffectManagerV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &ExtBackgroundEffectManagerV1,
        event: ext_background_effect_manager_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let ext_background_effect_manager_v1::Event::Capabilities { flags } = event {
            state.capabilities = match flags {
                WEnum::Value(flags) => flags.bits(),
                WEnum::Unknown(bits) => bits,
            };
        }
    }
}

impl Dispatch<ExtBackgroundEffectSurfaceV1, ()> for State {
    fn event(
        _: &mut Self,
        _: &ExtBackgroundEffectSurfaceV1,
        _: ext_background_effect_surface_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_compositor::WlCompositor, ()> for State {
    fn event(
        _: &mut Self,
        _: &wl_compositor::WlCompositor,
        _: wl_compositor::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_region::WlRegion, ()> for State {
    fn event(
        _: &mut Self,
        _: &wl_region::WlRegion,
        _: wl_region::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The pixels a region's operations leave, as a grid.
    fn paint(ops: &[RegionOp], width: i32, height: i32) -> Vec<Vec<bool>> {
        let mut grid = vec![vec![false; width as usize]; height as usize];
        for op in ops {
            let (r, on) = match op {
                RegionOp::Add(r) => (r, true),
                RegionOp::Subtract(r) => (r, false),
            };
            for y in r.y..r.y + r.height {
                for x in r.x..r.x + r.width {
                    grid[y as usize][x as usize] = on;
                }
            }
        }
        grid
    }

    #[test]
    fn a_square_region_has_nothing_taken_away() {
        let region = Rect {
            x: 0,
            y: 0,
            width: 8,
            height: 6,
        };
        assert_eq!(rounded_region(region, 0), [RegionOp::Add(region)]);
    }

    #[test]
    fn the_corners_are_cut_as_the_cpp_cuts_them() {
        let ops = rounded_region(
            Rect {
                x: 0,
                y: 0,
                width: 10,
                height: 10,
            },
            4,
        );
        let grid = paint(&ops, 10, 10);
        // Row by row the C++ cuts r - floor(sqrt(r² - (r - i)²)): 4, 2, 1, 1.
        let row = |y: usize| grid[y].iter().filter(|on| **on).count();
        assert_eq!([row(0), row(1), row(2), row(3), row(4)], [2, 6, 8, 8, 10]);
        assert_eq!([row(9), row(8), row(7), row(6)], [2, 6, 8, 8], "symmetric");
        assert!(!grid[0][0] && !grid[0][9] && !grid[9][0] && !grid[9][9]);
        assert!(grid[0][4] && grid[5][0]);
    }

    #[test]
    fn a_region_off_the_origin_is_cut_where_it_is() {
        let ops = rounded_region(
            Rect {
                x: 3,
                y: 2,
                width: 6,
                height: 5,
            },
            2,
        );
        assert!(ops.contains(&RegionOp::Subtract(Rect {
            x: 3,
            y: 2,
            width: 2,
            height: 1
        })));
        assert!(ops.contains(&RegionOp::Subtract(Rect {
            x: 7,
            y: 6,
            width: 2,
            height: 1
        })));
    }

    #[test]
    fn blur_is_the_capability_bit() {
        assert!(supports_blur(1));
        assert!(supports_blur(3));
        assert!(!supports_blur(0));
        assert!(!supports_blur(2), "an unknown capability is not blur");
    }
}
