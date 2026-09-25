//! The bridge against a real compositor: headless Sway.
//!
//! A surface is made on one libwayland connection, as a toolkit makes the
//! launcher's; its raw pointers are handed over the way `raw-window-handle`
//! hands them out; and the bridge's connection and proxy are then used for
//! real requests. The harness is `compass-wayland`'s (one Sway per test,
//! skipped loudly without `sway`, required when `COMPASS_REQUIRE_COMPOSITOR`
//! is set).
//!
//! What this cannot show is blur itself: Sway has no
//! `ext_background_effect_manager_v1`, so the blur client is shown to report
//! that, without an error, on the bridged connection. KWin and niri, which
//! blur, are the VM tier.

#[path = "../../compass-wayland/tests/support/mod.rs"]
mod support;

use std::ffi::c_void;
use std::ptr::NonNull;

use compass_wayland::compositor;
use compass_wayland::material::{Applied, BackgroundEffects, MaterialError, Params, Rect};
use compass_wayland_foreign::{BridgeError, bridge};
use raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, WaylandDisplayHandle,
    WaylandWindowHandle, WindowHandle,
};
use support::Sway;
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::{wl_compositor, wl_registry, wl_surface};
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle};

/// A toolkit's window, as far as `raw-window-handle` is concerned: the two
/// pointers, lent out while it is borrowed.
struct ToolkitWindow {
    display: NonNull<c_void>,
    surface: NonNull<c_void>,
}

impl ToolkitWindow {
    fn of(connection: &Connection, proxy: &impl Proxy) -> Self {
        Self {
            display: NonNull::new(connection.backend().display_ptr().cast())
                .expect("a connected display"),
            surface: NonNull::new(proxy.id().as_ptr().cast()).expect("a live proxy"),
        }
    }
}

// The one place outside `adopt` that needs `unsafe`: forging the borrowed
// handles a toolkit would lend. Each borrow is of a pointer the test's own
// connection keeps alive for the whole test.
#[allow(unsafe_code)]
impl HasDisplayHandle for ToolkitWindow {
    fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
        let raw = WaylandDisplayHandle::new(self.display).into();
        // SAFETY: the display is the test connection's, open until the test ends.
        Ok(unsafe { DisplayHandle::borrow_raw(raw) })
    }
}

#[allow(unsafe_code)]
impl HasWindowHandle for ToolkitWindow {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        let raw = WaylandWindowHandle::new(self.surface).into();
        // SAFETY: the proxy is the test's, alive while the window is borrowed.
        Ok(unsafe { WindowHandle::borrow_raw(raw) })
    }
}

/// The toolkit's side: its connection, its queue and its surface.
struct Toolkit {
    connection: Connection,
    queue: EventQueue<Objects>,
    compositor: wl_compositor::WlCompositor,
    surface: wl_surface::WlSurface,
}

impl Toolkit {
    fn on(sway: &Sway) -> Self {
        let connection = sway.connect();
        let (globals, mut queue) = registry_queue_init::<Objects>(&connection).expect("registry");
        let qh = queue.handle();
        let compositor: wl_compositor::WlCompositor =
            globals.bind(&qh, 1..=6, ()).expect("wl_compositor");
        let surface = compositor.create_surface(&qh, ());
        queue.roundtrip(&mut Objects).expect("the surface exists");
        Self {
            connection,
            queue,
            compositor,
            surface,
        }
    }
}

#[test]
fn a_toolkits_surface_is_bridged_into_a_usable_proxy() {
    let Some(sway) = Sway::start("a_toolkits_surface_is_bridged_into_a_usable_proxy") else {
        return;
    };
    let mut toolkit = Toolkit::on(&sway);
    let window = ToolkitWindow::of(&toolkit.connection, &toolkit.surface);

    let bridged = bridge(&window).expect("a wayland-rs surface of a live display");
    assert_eq!(
        bridged.connection.backend().display_ptr(),
        toolkit.connection.backend().display_ptr(),
        "the bridge speaks over the toolkit's display, not a connection of its own"
    );
    assert_eq!(
        bridged.surface.id(),
        toolkit.surface.id(),
        "the same object"
    );
    assert!(bridged.surface.is_alive());

    // Requests through the bridged proxy reach the compositor as the
    // toolkit's own would; a bad one would be a protocol error on the shared
    // display, which both roundtrips below would report.
    let (globals, mut ours) =
        registry_queue_init::<Objects>(&bridged.connection).expect("a registry on the bridge");
    let compositor: wl_compositor::WlCompositor = globals
        .bind(&ours.handle(), 1..=6, ())
        .expect("wl_compositor on the bridge");
    let region = compositor.create_region(&ours.handle(), ());
    region.add(0, 0, 32, 32);
    bridged.surface.set_input_region(Some(&region));
    bridged.surface.commit();
    region.destroy();
    ours.roundtrip(&mut Objects)
        .expect("the bridged connection is sound");
    toolkit
        .queue
        .roundtrip(&mut Objects)
        .expect("and the toolkit's is untouched");

    // A second bridge of the same display reuses the one connection.
    let again = bridge(&window).expect("bridged again");
    assert!(again.connection.backend() == bridged.connection.backend());

    // The proxy follows the toolkit's surface: once the toolkit destroys it,
    // the bridged proxy knows, and a request through it is refused rather
    // than sent to a freed object.
    toolkit.surface.destroy();
    toolkit.queue.roundtrip(&mut Objects).expect("destroyed");
    assert!(!bridged.surface.is_alive());
    assert!(!again.surface.is_alive());
}

#[test]
fn a_pointer_that_is_not_a_surface_is_refused() {
    let Some(sway) = Sway::start("a_pointer_that_is_not_a_surface_is_refused") else {
        return;
    };
    let toolkit = Toolkit::on(&sway);
    let window = ToolkitWindow::of(&toolkit.connection, &toolkit.compositor);
    assert!(matches!(bridge(&window), Err(BridgeError::NotASurface)));
}

#[test]
fn on_sway_the_bridged_connection_reports_no_blur_without_an_error() {
    use wayland_client::protocol::wl_surface::WlSurface;

    let Some(sway) = Sway::start("on_sway_the_bridged_connection_reports_no_blur") else {
        return;
    };
    let toolkit = Toolkit::on(&sway);
    let bridged =
        bridge(&ToolkitWindow::of(&toolkit.connection, &toolkit.surface)).expect("bridged");
    let advertised = compositor::probe_connection(&bridged.connection)
        .expect("the registry, over the bridge")
        .names()
        .any(|name| name == "ext_background_effect_manager_v1");
    let apply = |effects: &mut BackgroundEffects, surface: &WlSurface| {
        effects.apply(
            surface,
            Params {
                radius: 16,
                region: Rect {
                    x: 24,
                    y: 24,
                    width: 768,
                    height: 560,
                },
            },
        )
    };
    match BackgroundEffects::bind(&bridged.connection) {
        Err(MaterialError::Unsupported) => assert!(!advertised, "advertised but refused"),
        Err(other) => panic!("{other}"),
        Ok(mut effects) => {
            // A Sway that has grown the protocol: the bridged surface takes it.
            assert!(advertised);
            let applied = apply(&mut effects, &bridged.surface);
            assert!(matches!(applied, Applied::Created | Applied::Unsupported));
            effects.flush().expect("sent");
        }
    }
    let mut queue = toolkit.queue;
    queue
        .roundtrip(&mut Objects)
        .expect("no protocol error reached the toolkit's display");
}

struct Objects;

wayland_client::delegate_noop!(Objects: wl_compositor::WlCompositor);
wayland_client::delegate_noop!(Objects: ignore wl_surface::WlSurface);
wayland_client::delegate_noop!(Objects: wayland_client::protocol::wl_region::WlRegion);

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for Objects {
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
