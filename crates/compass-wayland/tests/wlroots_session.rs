//! The wlroots track against a real compositor: headless Sway.
//!
//! These are the tests `PLAN.md` §8.4 (c) deferred "until there is wlroots
//! code to test", with one change of plan: a real Sway rather than a
//! `smithay` mock. A mock written alongside the client proves the two agree
//! with each other; Sway proves the client agrees with a compositor people
//! run. See `support` for how each test gets its own Sway and how a missing
//! one is reported.

mod support;

use std::io::Read;
use std::time::Duration;

use compass_wayland::clipboard::{self, SelectionChange};
use compass_wayland::compositor::{self, Family};
use compass_wayland::data_control::{Offer, PASSWORD_HINT_MIME_TYPE};
use compass_wayland::hotkey::{self, HotkeyClient, HotkeyError};
use compass_wayland::toplevel::{Source, ToplevelError, Toplevels};
use support::{Sway, TestWindow, child_role, eventually};

const WAIT: Duration = Duration::from_secs(10);

#[test]
fn sway_is_the_wlroots_family_with_the_protocols_the_track_uses() {
    let Some(sway) = Sway::start("sway_is_the_wlroots_family") else {
        return;
    };
    let globals = compositor::probe_connection(&sway.connect()).expect("the registry");
    assert_eq!(compositor::family(Some("sway"), &globals), Family::Wlroots);
    assert_eq!(compositor::family(None, &globals), Family::Wlroots);
    // GNOME stays GNOME even against a registry that has everything.
    assert_eq!(compositor::family(Some("GNOME"), &globals), Family::Gnome);

    let caps = compositor::Capabilities::of(&globals);
    assert!(
        caps.layer_shell,
        "{:?}",
        globals.names().collect::<Vec<_>>()
    );
    assert!(caps.toplevel_management);
    assert!(caps.data_control);
}

#[test]
fn windows_are_listed_focused_and_closed() {
    let Some(sway) = Sway::start("windows_are_listed_focused_and_closed") else {
        return;
    };
    let alpha = TestWindow::open(&sway, "Alpha document", "test.Alpha");
    let beta = TestWindow::open(&sway, "Beta document", "test.Beta");

    let toplevels = Toplevels::connect_to(sway.connect()).expect("the toplevel manager");
    assert_eq!(toplevels.source(), Source::WlrManagement);
    assert!(
        eventually(WAIT, || toplevels.list().len() == 2),
        "{:?}",
        toplevels.list()
    );

    let list = toplevels.list();
    let find = |app: &str| list.iter().find(|t| t.app_id == app).cloned().unwrap();
    let (a, b) = (find("test.Alpha"), find("test.Beta"));
    assert_eq!(a.title, "Alpha document");
    assert_eq!(b.title, "Beta document");
    assert!(a.can_act && b.can_act);
    // Sway focuses the window mapped last.
    assert!(b.activated && !a.activated, "{list:?}");
    assert_eq!(list[0].app_id, "test.Beta", "most recently active first");

    toplevels.activate(a.id).expect("activating alpha");
    assert!(
        eventually(WAIT, || {
            let list = toplevels.list();
            list.first().is_some_and(|t| t.id == a.id && t.activated)
        }),
        "alpha never became active and first: {:?}",
        toplevels.list()
    );

    toplevels.close(b.id).expect("closing beta");
    assert!(
        eventually(WAIT, || beta.is_closed()),
        "beta was never asked to close"
    );
    assert!(
        eventually(WAIT, || toplevels.list().iter().all(|t| t.id != b.id)),
        "a closed window stayed listed: {:?}",
        toplevels.list()
    );
    assert!(matches!(
        toplevels.activate(b.id),
        Err(ToplevelError::NoSuchWindow(_))
    ));
    drop(alpha);
}

#[test]
fn a_change_to_the_window_set_is_announced() {
    let Some(sway) = Sway::start("a_change_to_the_window_set_is_announced") else {
        return;
    };
    let toplevels = Toplevels::connect_to(sway.connect()).expect("the toplevel manager");
    let changes = toplevels.changes();
    let before = *changes.borrow();
    let _window = TestWindow::open(&sway, "Gamma", "test.Gamma");
    assert!(eventually(WAIT, || *changes.borrow() != before));
    assert!(eventually(WAIT, || toplevels.list().len() == 1));
}

#[test]
fn the_headless_output_is_listed_with_its_name_and_mode() {
    let Some(sway) = Sway::start("the_headless_output_is_listed") else {
        return;
    };
    let outputs = compass_wayland::output::list_on(&sway.connect()).expect("the outputs");
    assert_eq!(outputs.len(), 1, "{outputs:?}");
    let output = &outputs[0];
    assert_eq!(output.name, "HEADLESS-1");
    assert_eq!((output.pixel_width, output.pixel_height), (1280, 800));
    assert_eq!(
        (output.x, output.y, output.width, output.height),
        (0, 0, 1280, 800)
    );
}

/// Child role: select `COMPASS_WLR_TEXT` (the primary selection), then read
/// it back as an extension's `getSelectedText` would.
#[test]
fn child_selects_text() {
    if child_role().is_none() {
        return;
    }
    let text = std::env::var("COMPASS_WLR_TEXT").unwrap();
    assert_eq!(
        clipboard::read_primary_text().expect("nothing selected yet"),
        None
    );
    let mut options = wl_clipboard_rs::copy::Options::new();
    options.clipboard(wl_clipboard_rs::copy::ClipboardType::Primary);
    options
        .copy(
            wl_clipboard_rs::copy::Source::Bytes(text.clone().into_bytes().into_boxed_slice()),
            wl_clipboard_rs::copy::MimeType::Text,
        )
        .expect("selecting");
    assert!(
        eventually(WAIT, || clipboard::read_primary_text()
            .ok()
            .flatten()
            .is_some_and(|read| read == text)),
        "the selection never read back"
    );
    assert_eq!(
        clipboard::read("text/plain").expect("the clipboard"),
        None,
        "selecting is not copying"
    );
    println!("CHILD-OK");
}

#[test]
fn the_primary_selection_reads_back_and_is_not_the_clipboard() {
    let Some(sway) = Sway::start("the_primary_selection_reads_back") else {
        return;
    };
    let child = sway.run_child(
        "child_selects_text",
        "select",
        &[("COMPASS_WLR_TEXT", "selected words")],
    );
    let output = child.wait_with_output().expect("the child");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success() && stdout.contains("CHILD-OK"),
        "child failed:\n{stdout}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Child role: put `COMPASS_WLR_TEXT` on the clipboard through
/// [`clipboard::set`], read it back through [`clipboard::read`], and keep
/// serving it long enough for the parent's watcher.
#[test]
fn child_sets_the_clipboard() {
    let Some(role) = child_role() else { return };
    let text = std::env::var("COMPASS_WLR_TEXT").unwrap();
    let mut offers = vec![Offer {
        mime_type: "text/plain;charset=utf-8".to_owned(),
        data: text.clone().into_bytes(),
    }];
    if role == "conceal" {
        offers.push(Offer {
            mime_type: PASSWORD_HINT_MIME_TYPE.to_owned(),
            data: b"secret".to_vec(),
        });
    }
    clipboard::set(offers).expect("setting the clipboard");
    let back = clipboard::read("text/plain").expect("reading it back");
    assert_eq!(back.as_deref(), Some(text.as_bytes()));
    let types = clipboard::mime_types().expect("its types");
    assert!(
        types.iter().any(|t| t == "text/plain;charset=utf-8"),
        "{types:?}"
    );
    println!("CHILD-OK");
    // The selection is served from this process; it has to outlive the
    // parent's read.
    std::thread::sleep(Duration::from_secs(4));
}

fn watch_and_copy(role: &str, text: &str) -> (SelectionChange, String) {
    let sway = Sway::start("clipboard").expect("checked by the caller");
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let watcher = clipboard::watch_on(sway.connect(), tx).expect("a data-control device");
    assert_eq!(watcher.protocol(), "zwlr_data_control_manager_v1");

    let mut child = sway.run_child(
        "child_sets_the_clipboard",
        role,
        &[("COMPASS_WLR_TEXT", text)],
    );
    let deadline = std::time::Instant::now() + WAIT;
    let change = loop {
        match rx.try_recv() {
            Ok(change) => break change,
            Err(_) if std::time::Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(err) => panic!("no selection change arrived: {err}"),
        }
    };
    let status = child.wait().expect("the child");
    let mut out = String::new();
    child
        .stdout
        .take()
        .unwrap()
        .read_to_string(&mut out)
        .unwrap();
    let mut err = String::new();
    child
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut err)
        .unwrap();
    assert!(status.success(), "child failed:\n{out}\n{err}");
    (change, out)
}

#[test]
fn a_copy_is_seen_by_the_watcher_and_round_trips_through_wl_clipboard_rs() {
    if Sway::start("a_copy_is_seen_by_the_watcher").is_none() {
        return;
    }
    let (change, out) = watch_and_copy("plain", "hello from compass");
    assert!(out.contains("CHILD-OK"), "{out}");
    assert!(!change.concealed());
    let preferred = change.preferred().expect("an offer to record");
    assert_eq!(preferred.mime_type, "text/plain;charset=utf-8");
    assert_eq!(preferred.data, b"hello from compass");
}

#[test]
fn a_password_manager_copy_is_marked_concealed() {
    if Sway::start("a_password_manager_copy_is_marked_concealed").is_none() {
        return;
    }
    let (change, _) = watch_and_copy("conceal", "hunter2");
    assert!(change.concealed(), "{change:?}");
    // The flag's bytes are never read, only its presence recorded.
    let flag = change
        .offers
        .iter()
        .find(|o| o.mime_type == PASSWORD_HINT_MIME_TYPE)
        .unwrap();
    assert!(flag.data.is_empty());
}

#[test]
fn a_compositor_without_xx_hotkey_says_so_and_the_fallback_names_the_command() {
    let Some(sway) = Sway::start("a_compositor_without_xx_hotkey") else {
        return;
    };
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    match HotkeyClient::connect_on(sway.connect(), "com.vicinae.Vicinae", tx) {
        Err(HotkeyError::Unsupported) => {}
        other => panic!("Sway 1.x has neither hotkey protocol, got {other:?}"),
    }
    assert!(hotkey::manual_binding_hint(Some("sway")).contains("vicinae toggle"));
}

#[test]
fn background_effect_is_bound_where_advertised_and_refused_by_name_where_not() {
    use compass_wayland::material::{Applied, BackgroundEffects, MaterialError, Params, Rect};
    use wayland_client::protocol::wl_compositor::WlCompositor;
    let Some(sway) = Sway::start("background_effect") else {
        return;
    };
    let connection = sway.connect();
    let advertised = compositor::probe_connection(&connection)
        .expect("the registry")
        .names()
        .any(|name| name == "ext_background_effect_manager_v1");
    match BackgroundEffects::bind(&connection) {
        Err(MaterialError::Unsupported) => {
            assert!(!advertised, "the manager is there but was refused");
        }
        Err(other) => panic!("{other}"),
        Ok(mut effects) => {
            assert!(advertised);
            // A surface of this connection, as the launcher's would be.
            let (globals, queue) =
                wayland_client::globals::registry_queue_init::<Surfaces>(&connection).unwrap();
            let compositor = globals
                .bind::<WlCompositor, _, _>(&queue.handle(), 1..=6, ())
                .unwrap();
            let surface = compositor.create_surface(&queue.handle(), ());
            let params = Params {
                radius: 10,
                region: Rect {
                    x: 0,
                    y: 0,
                    width: 640,
                    height: 480,
                },
            };
            let first = effects.apply(&surface, params);
            if effects.supports_blur() {
                assert_eq!(first, Applied::Created);
                assert_eq!(effects.apply(&surface, params), Applied::Unchanged);
                assert!(effects.clear(&surface));
            } else {
                assert_eq!(first, Applied::Unsupported);
            }
        }
    }
}

struct Surfaces;

wayland_client::delegate_noop!(Surfaces: wayland_client::protocol::wl_compositor::WlCompositor);
wayland_client::delegate_noop!(Surfaces: ignore wayland_client::protocol::wl_surface::WlSurface);

impl
    wayland_client::Dispatch<
        wayland_client::protocol::wl_registry::WlRegistry,
        wayland_client::globals::GlobalListContents,
    > for Surfaces
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
