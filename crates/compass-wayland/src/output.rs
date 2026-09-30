//! The monitors, as the compositor describes them: `wl_output`, with
//! `zxdg_output_manager_v1` for the logical layout when it is advertised.
//!
//! Any compositor carries `wl_output` — Mutter and every wlroots one — so this
//! is not a wlroots-only path. It is what an extension's
//! `WindowManagement.getScreens` answers, the counterpart of the C++ reading
//! `QGuiApplication::screens()` and then correcting the resolution from the
//! current output mode, because the scale-derived size is wrong under
//! fractional scaling.
//!
//! A short-lived connection per call: the list is asked for rarely, and a
//! snapshot is what the caller wants.

use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::{wl_output, wl_registry};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle, WEnum};
use wayland_protocols::xdg::xdg_output::zv1::client::{zxdg_output_manager_v1, zxdg_output_v1};

/// The highest `wl_output` version this reads (`name` and `description`
/// arrived in 4).
const WL_OUTPUT_VERSION: u32 = 4;

/// One monitor.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Output {
    /// The connector name (`DP-1`), from `wl_output.name`; empty before
    /// version 4.
    pub name: String,
    /// The manufacturer, from `wl_output.geometry`.
    pub make: String,
    /// The model, from `wl_output.geometry`.
    pub model: String,
    /// Its place and size in the logical layout.
    pub x: i32,
    /// See [`Self::x`].
    pub y: i32,
    /// See [`Self::x`].
    pub width: i32,
    /// See [`Self::x`].
    pub height: i32,
    /// The current mode, in device pixels.
    pub pixel_width: i32,
    /// See [`Self::pixel_width`].
    pub pixel_height: i32,
}

/// Why the outputs could not be read.
#[derive(Debug, thiserror::Error)]
pub enum OutputError {
    /// No compositor to connect to.
    #[error("no Wayland compositor: {0}")]
    Connect(#[from] wayland_client::ConnectError),
    /// The connection failed.
    #[error("Wayland connection: {0}")]
    Connection(String),
}

/// The monitors of the compositor `$WAYLAND_DISPLAY` names.
///
/// # Errors
///
/// [`OutputError`] when there is no compositor or it stops answering.
pub fn list() -> Result<Vec<Output>, OutputError> {
    list_on(&Connection::connect_to_env()?)
}

/// [`list`], over an existing connection.
///
/// # Errors
///
/// [`OutputError::Connection`] when a round trip fails.
pub fn list_on(connection: &Connection) -> Result<Vec<Output>, OutputError> {
    let failed = |err: &dyn std::fmt::Display| OutputError::Connection(err.to_string());
    let (globals, mut queue) =
        registry_queue_init::<State>(connection).map_err(|err| failed(&err))?;
    let handle = queue.handle();
    let mut state = State::default();
    let mut outputs: Vec<wl_output::WlOutput> = Vec::new();
    let mut xdg_outputs: Vec<zxdg_output_v1::ZxdgOutputV1> = Vec::new();
    for global in globals.contents().clone_list() {
        if global.interface == wl_output::WlOutput::interface().name {
            let output: wl_output::WlOutput = globals.registry().bind(
                global.name,
                global.version.min(WL_OUTPUT_VERSION),
                &handle,
                state.entries.len(),
            );
            outputs.push(output);
            state.entries.push(Entry::default());
        }
    }
    let manager: Option<zxdg_output_manager_v1::ZxdgOutputManagerV1> =
        globals.bind(&handle, 1..=3, ()).ok();
    if let Some(manager) = &manager {
        for (index, output) in outputs.iter().enumerate() {
            xdg_outputs.push(manager.get_xdg_output(output, &handle, index));
        }
    }
    // Two round trips: the first delivers the binds' initial events, the
    // second whatever those events caused.
    queue.roundtrip(&mut state).map_err(|err| failed(&err))?;
    queue.roundtrip(&mut state).map_err(|err| failed(&err))?;

    for xdg in xdg_outputs {
        xdg.destroy();
    }
    for output in outputs {
        if output.version() >= 3 {
            output.release();
        }
    }
    if let Some(manager) = manager {
        manager.destroy();
    }
    Ok(state.entries.iter().map(Entry::describe).collect())
}

#[derive(Default)]
struct State {
    entries: Vec<Entry>,
}

/// What one output's events said.
#[derive(Default)]
struct Entry {
    name: String,
    make: String,
    model: String,
    position: (i32, i32),
    transform_swaps: bool,
    scale: i32,
    mode: (i32, i32),
    logical_position: Option<(i32, i32)>,
    logical_size: Option<(i32, i32)>,
}

impl Entry {
    fn describe(&self) -> Output {
        let (pixel_width, pixel_height) = self.mode;
        let (x, y) = self.logical_position.unwrap_or(self.position);
        let (width, height) = self.logical_size.unwrap_or_else(|| {
            let scale = self.scale.max(1);
            let (w, h) = (pixel_width / scale, pixel_height / scale);
            if self.transform_swaps { (h, w) } else { (w, h) }
        });
        Output {
            name: self.name.clone(),
            make: self.make.clone(),
            model: self.model.clone(),
            x,
            y,
            width,
            height,
            pixel_width,
            pixel_height,
        }
    }
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

impl Dispatch<wl_output::WlOutput, usize> for State {
    fn event(
        state: &mut Self,
        _: &wl_output::WlOutput,
        event: wl_output::Event,
        index: &usize,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some(entry) = state.entries.get_mut(*index) else {
            return;
        };
        match event {
            wl_output::Event::Geometry {
                x,
                y,
                make,
                model,
                transform,
                ..
            } => {
                entry.position = (x, y);
                entry.make = make;
                entry.model = model;
                entry.transform_swaps = matches!(
                    transform,
                    WEnum::Value(
                        wl_output::Transform::_90
                            | wl_output::Transform::_270
                            | wl_output::Transform::Flipped90
                            | wl_output::Transform::Flipped270
                    )
                );
            }
            wl_output::Event::Mode {
                flags,
                width,
                height,
                ..
            } => {
                let current = match flags {
                    WEnum::Value(flags) => flags.contains(wl_output::Mode::Current),
                    WEnum::Unknown(bits) => bits & 1 != 0,
                };
                if current {
                    entry.mode = (width, height);
                }
            }
            wl_output::Event::Scale { factor } => entry.scale = factor,
            wl_output::Event::Name { name } => entry.name = name,
            _ => {}
        }
    }
}

impl Dispatch<zxdg_output_manager_v1::ZxdgOutputManagerV1, ()> for State {
    fn event(
        _: &mut Self,
        _: &zxdg_output_manager_v1::ZxdgOutputManagerV1,
        _: zxdg_output_manager_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<zxdg_output_v1::ZxdgOutputV1, usize> for State {
    fn event(
        state: &mut Self,
        _: &zxdg_output_v1::ZxdgOutputV1,
        event: zxdg_output_v1::Event,
        index: &usize,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some(entry) = state.entries.get_mut(*index) else {
            return;
        };
        match event {
            zxdg_output_v1::Event::LogicalPosition { x, y } => {
                entry.logical_position = Some((x, y));
            }
            zxdg_output_v1::Event::LogicalSize { width, height } => {
                entry.logical_size = Some((width, height));
            }
            // `wl_output.name` is the one to trust from version 4; before it,
            // this is the only name there is.
            zxdg_output_v1::Event::Name { name } if entry.name.is_empty() => entry.name = name,
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry() -> Entry {
        Entry {
            mode: (3840, 2160),
            scale: 2,
            position: (1920, 0),
            ..Entry::default()
        }
    }

    #[test]
    fn without_xdg_output_the_layout_is_the_mode_over_the_scale() {
        let output = entry().describe();
        assert_eq!((output.x, output.y), (1920, 0));
        assert_eq!((output.width, output.height), (1920, 1080));
        assert_eq!((output.pixel_width, output.pixel_height), (3840, 2160));
    }

    #[test]
    fn a_rotated_output_swaps_its_logical_size() {
        let output = Entry {
            transform_swaps: true,
            ..entry()
        }
        .describe();
        assert_eq!((output.width, output.height), (1080, 1920));
    }

    #[test]
    fn xdg_output_wins_because_fractional_scales_round_the_integer_one() {
        let output = Entry {
            logical_position: Some((0, 0)),
            logical_size: Some((2560, 1440)),
            ..entry()
        }
        .describe();
        assert_eq!((output.x, output.width, output.height), (0, 2560, 1440));
    }
}
