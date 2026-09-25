//! `compass_wayland::material` against a fake compositor that blurs.
//!
//! Sway has no `ext_background_effect_manager_v1`, so the headless-Sway test
//! only sees the refusal. Here the compositor side of the protocol is stood
//! up in-process with `wayland-server` over a socket pair — real wire
//! traffic, nothing reaching a real display — and it records what the client
//! asks for. It also enforces the one error that matters to the launcher:
//! `set_blur_region` on an effect whose surface is gone is
//! `surface_destroyed`, which on the toolkit's shared display would end the
//! launcher. The toolkit destroys the launcher's surface at every hide.

use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use compass_wayland::material::{Applied, BackgroundEffects, Params, Rect};
use wayland_client::protocol::wl_compositor::WlCompositor as ClientCompositor;
use wayland_client::protocol::wl_surface::WlSurface as ClientSurface;
use wayland_protocols::ext::background_effect::v1::server::{
    ext_background_effect_manager_v1::{self, ExtBackgroundEffectManagerV1},
    ext_background_effect_surface_v1::{self, ExtBackgroundEffectSurfaceV1},
};
use wayland_server::backend::ClientData;
use wayland_server::protocol::{wl_compositor, wl_region, wl_surface};
use wayland_server::{
    Client, DataInit, Dispatch, Display, DisplayHandle, GlobalDispatch, New, Resource,
};

#[derive(Default)]
struct Compositor {
    log: Arc<Mutex<Vec<String>>>,
    blur: bool,
}

impl Compositor {
    fn log(&self, line: impl Into<String>) {
        self.log.lock().unwrap().push(line.into());
    }
}

struct NoData;
impl ClientData for NoData {}

impl GlobalDispatch<wl_compositor::WlCompositor, ()> for Compositor {
    fn bind(
        _: &mut Self,
        _: &DisplayHandle,
        _: &Client,
        resource: New<wl_compositor::WlCompositor>,
        _: &(),
        data_init: &mut DataInit<'_, Self>,
    ) {
        data_init.init(resource, ());
    }
}

impl Dispatch<wl_compositor::WlCompositor, ()> for Compositor {
    fn request(
        _: &mut Self,
        _: &Client,
        _: &wl_compositor::WlCompositor,
        request: wl_compositor::Request,
        _: &(),
        _: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        match request {
            wl_compositor::Request::CreateSurface { id } => {
                data_init.init(id, ());
            }
            wl_compositor::Request::CreateRegion { id } => {
                data_init.init(id, ());
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_surface::WlSurface, ()> for Compositor {
    fn request(
        state: &mut Self,
        _: &Client,
        _: &wl_surface::WlSurface,
        request: wl_surface::Request,
        _: &(),
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
        if let wl_surface::Request::Destroy = request {
            state.log("surface destroyed");
        }
    }
}

impl Dispatch<wl_region::WlRegion, ()> for Compositor {
    fn request(
        state: &mut Self,
        _: &Client,
        _: &wl_region::WlRegion,
        request: wl_region::Request,
        _: &(),
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
        if let wl_region::Request::Add {
            x,
            y,
            width,
            height,
        } = request
        {
            state.log(format!("region add {x} {y} {width} {height}"));
        }
    }
}

impl GlobalDispatch<ExtBackgroundEffectManagerV1, ()> for Compositor {
    fn bind(
        state: &mut Self,
        _: &DisplayHandle,
        _: &Client,
        resource: New<ExtBackgroundEffectManagerV1>,
        _: &(),
        data_init: &mut DataInit<'_, Self>,
    ) {
        let manager = data_init.init(resource, ());
        manager.capabilities(if state.blur {
            ext_background_effect_manager_v1::Capability::Blur
        } else {
            ext_background_effect_manager_v1::Capability::empty()
        });
    }
}

impl Dispatch<ExtBackgroundEffectManagerV1, ()> for Compositor {
    fn request(
        state: &mut Self,
        _: &Client,
        _: &ExtBackgroundEffectManagerV1,
        request: ext_background_effect_manager_v1::Request,
        _: &(),
        _: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        if let ext_background_effect_manager_v1::Request::GetBackgroundEffect { id, surface } =
            request
        {
            state.log("effect created");
            data_init.init(id, surface);
        }
    }
}

impl Dispatch<ExtBackgroundEffectSurfaceV1, wl_surface::WlSurface> for Compositor {
    fn request(
        state: &mut Self,
        _: &Client,
        effect: &ExtBackgroundEffectSurfaceV1,
        request: ext_background_effect_surface_v1::Request,
        surface: &wl_surface::WlSurface,
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
        match request {
            ext_background_effect_surface_v1::Request::SetBlurRegion { .. } => {
                if surface.is_alive() {
                    state.log("blur region set");
                } else {
                    state.log("PROTOCOL ERROR surface_destroyed");
                    effect.post_error(
                        ext_background_effect_surface_v1::Error::SurfaceDestroyed,
                        "the surface is gone",
                    );
                }
            }
            ext_background_effect_surface_v1::Request::Destroy => state.log("effect destroyed"),
            _ => {}
        }
    }
}

/// A fake compositor on its own thread, and the client end of its socket.
struct Fake {
    log: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Fake {
    fn start(blur: bool) -> (Self, wayland_client::Connection) {
        let (server, client) = UnixStream::pair().expect("a socket pair");
        let mut display: Display<Compositor> = Display::new().expect("a display");
        let mut handle = display.handle();
        handle.create_global::<Compositor, wl_compositor::WlCompositor, ()>(6, ());
        handle.create_global::<Compositor, ExtBackgroundEffectManagerV1, ()>(1, ());
        handle
            .insert_client(server, Arc::new(NoData))
            .expect("the client");
        let log = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let thread = {
            let log = Arc::clone(&log);
            let stop = Arc::clone(&stop);
            std::thread::spawn(move || {
                let mut state = Compositor { log, blur };
                while !stop.load(Ordering::Acquire) {
                    let _ = display.dispatch_clients(&mut state);
                    let _ = display.flush_clients();
                    std::thread::sleep(Duration::from_millis(1));
                }
            })
        };
        let connection =
            wayland_client::Connection::from_socket(client).expect("a client connection");
        (
            Self {
                log,
                stop,
                thread: Some(thread),
            },
            connection,
        )
    }

    fn log(&self) -> Vec<String> {
        self.log.lock().unwrap().clone()
    }
}

impl Drop for Fake {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// The toolkit's side of the connection: a queue and its surfaces.
struct Toolkit {
    queue: wayland_client::EventQueue<Objects>,
    compositor: ClientCompositor,
}

impl Toolkit {
    fn on(connection: &wayland_client::Connection) -> Self {
        let (globals, queue) =
            wayland_client::globals::registry_queue_init::<Objects>(connection).expect("registry");
        let compositor = globals
            .bind(&queue.handle(), 1..=6, ())
            .expect("wl_compositor");
        Self { queue, compositor }
    }

    fn surface(&self) -> ClientSurface {
        self.compositor.create_surface(&self.queue.handle(), ())
    }

    fn roundtrip(&mut self) {
        self.queue
            .roundtrip(&mut Objects)
            .expect("no protocol error on the display");
    }
}

fn card(height: i32) -> Params {
    Params {
        radius: 16,
        region: Rect {
            x: 24,
            y: 24,
            width: 768,
            height,
        },
    }
}

#[test]
fn a_blurring_compositor_gets_the_region_once_per_change() {
    let (fake, connection) = Fake::start(true);
    let mut toolkit = Toolkit::on(&connection);
    let mut effects = BackgroundEffects::bind(&connection).expect("the manager");
    assert!(effects.supports_blur());
    let surface = toolkit.surface();

    assert_eq!(effects.apply(&surface, card(560)), Applied::Created);
    assert_eq!(effects.apply(&surface, card(560)), Applied::Unchanged);
    assert_eq!(effects.apply(&surface, card(320)), Applied::Updated);
    effects.flush().expect("sent");
    toolkit.roundtrip();

    let log = fake.log();
    assert_eq!(
        log.iter().filter(|line| *line == "blur region set").count(),
        2,
        "one region per change, none for the unchanged one: {log:?}"
    );
    assert_eq!(
        log.iter().filter(|line| *line == "effect created").count(),
        1
    );
    assert!(
        log.contains(&"region add 24 24 768 560".to_owned()),
        "{log:?}"
    );
    assert!(
        log.contains(&"region add 24 24 768 320".to_owned()),
        "{log:?}"
    );
}

#[test]
fn a_destroyed_surface_is_never_sent_a_region() {
    // The launcher hides by closing its window, and the toolkit destroys the
    // surface; the next show is a new surface. An effect left on the old one
    // must be let go, never updated.
    let (fake, connection) = Fake::start(true);
    let mut toolkit = Toolkit::on(&connection);
    let mut effects = BackgroundEffects::bind(&connection).expect("the manager");

    let first = toolkit.surface();
    assert_eq!(effects.apply(&first, card(560)), Applied::Created);
    effects.flush().expect("sent");
    toolkit.roundtrip();

    first.destroy();
    toolkit.roundtrip();
    assert_eq!(effects.apply(&first, card(320)), Applied::SurfaceGone);
    assert_eq!(
        effects.effect_count(),
        0,
        "the dead surface's effect is let go"
    );

    let second = toolkit.surface();
    assert_eq!(effects.apply(&second, card(560)), Applied::Created);
    effects.flush().expect("sent");
    toolkit.roundtrip();

    let log = fake.log();
    assert!(
        !log.iter().any(|line| line.starts_with("PROTOCOL ERROR")),
        "{log:?}"
    );
    let destroyed = log.iter().position(|line| line == "surface destroyed");
    let let_go = log.iter().position(|line| line == "effect destroyed");
    assert!(destroyed.is_some() && let_go > destroyed, "{log:?}");
    assert_eq!(
        log.iter().filter(|line| *line == "blur region set").count(),
        2,
        "{log:?}"
    );
}

#[test]
fn without_the_blur_capability_nothing_is_asked() {
    let (fake, connection) = Fake::start(false);
    let mut toolkit = Toolkit::on(&connection);
    let mut effects = BackgroundEffects::bind(&connection).expect("the manager is advertised");
    assert!(!effects.supports_blur());
    let surface = toolkit.surface();
    assert_eq!(effects.apply(&surface, card(560)), Applied::Unsupported);
    toolkit.roundtrip();
    assert!(fake.log().is_empty(), "{:?}", fake.log());
}

struct Objects;

wayland_client::delegate_noop!(Objects: ClientCompositor);
wayland_client::delegate_noop!(Objects: ignore ClientSurface);

impl
    wayland_client::Dispatch<
        wayland_client::protocol::wl_registry::WlRegistry,
        wayland_client::globals::GlobalListContents,
    > for Objects
{
    fn event(
        _: &mut Self,
        _: &wayland_client::protocol::wl_registry::WlRegistry,
        _: wayland_client::protocol::wl_registry::Event,
        _: &wayland_client::globals::GlobalListContents,
        _: &wayland_client::Connection,
        _: &wayland_client::QueueHandle<Self>,
    ) {
    }
}
