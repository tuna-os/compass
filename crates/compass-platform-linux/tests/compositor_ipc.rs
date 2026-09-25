//! The Hyprland and niri clients against fake sockets in a temporary
//! directory, replaying replies captured from the real compositors
//! (`tests/fixtures/`). No compositor is involved and nothing outside the
//! temporary directory is touched.

use std::path::Path;

use compass_platform_linux::compositor::hyprland::Hyprland;
use compass_platform_linux::compositor::niri::Niri;
use compass_platform_linux::compositor::{OwnWindows, Provider, WmBounds};
use compass_testkit::fake_compositor::{FakeSocket, Framing};

fn fixture(path: &str) -> String {
    std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(path),
    )
    .expect("fixture")
}

fn fake_hyprland(dir: &Path, dispatch: &'static str) -> (FakeSocket, Hyprland) {
    let path = FakeSocket::hyprland_path(dir, "v0.50_1700000000");
    let fake = FakeSocket::replaying(
        &path,
        Framing::Hyprland,
        vec![
            ("-j/clients".into(), fixture("hyprland/clients.json")),
            ("-j/workspaces".into(), fixture("hyprland/workspaces.json")),
            (
                "-j/activeworkspace".into(),
                fixture("hyprland/activeworkspace.json"),
            ),
            (
                "-j/activewindow".into(),
                fixture("hyprland/activewindow.json"),
            ),
            ("dispatch hl.dsp".into(), dispatch.into()),
            ("dispatch".into(), "ok".into()),
        ],
    );
    (fake, Hyprland::new(path))
}

#[test]
fn hyprland_lists_windows_with_their_pid_workspace_and_geometry() {
    let dir = tempfile::tempdir().unwrap();
    let (_fake, hyprland) = fake_hyprland(dir.path(), "ok");
    let windows = hyprland.windows().unwrap();
    assert_eq!(windows.len(), 3);
    let foot = &windows[1];
    assert_eq!(foot.id, "0x5581c8a4f310");
    assert_eq!(foot.wm_class, "foot");
    assert_eq!(foot.pid, Some(2398));
    assert_eq!(foot.workspace.as_deref(), Some("1"));
    assert_eq!(
        foot.bounds,
        Some(WmBounds {
            x: 1290,
            y: 42,
            width: 1260,
            height: 1388
        })
    );
    assert!(foot.focused, "the activewindow is marked focused");
    assert!(!windows[0].focused);
    assert!(windows[2].fullscreen, "fullscreen mode 2 is full-screen");
    assert!(!windows[0].fullscreen);
}

#[test]
fn hyprland_workspaces_and_the_active_one() {
    let dir = tempfile::tempdir().unwrap();
    let (_fake, hyprland) = fake_hyprland(dir.path(), "ok");
    let workspaces = hyprland.workspaces().unwrap();
    assert_eq!(
        workspaces
            .iter()
            .map(|w| (w.id.as_str(), w.name.as_str(), w.monitor.as_deref()))
            .collect::<Vec<_>>(),
        [("1", "1", Some("DP-1")), ("3", "music", Some("HDMI-A-1"))]
    );
    assert!(workspaces[1].has_fullscreen);
    assert_eq!(workspaces[1].number, Some(3));
    assert_eq!(
        hyprland.active_workspace().unwrap().map(|w| w.id),
        Some("1".into())
    );
}

#[test]
fn hyprland_frontmost_is_the_lowest_focus_history_on_the_active_workspace_not_the_launcher() {
    let dir = tempfile::tempdir().unwrap();
    let (_fake, hyprland) = fake_hyprland(dir.path(), "ok");
    let front = hyprland.frontmost_window(&OwnWindows::default()).unwrap();
    assert_eq!(front.map(|w| w.wm_class), Some("foot".into()));
    // With foot counted as the launcher, the window before it on the same
    // workspace; Spotify has a later history but is on another workspace.
    let own = OwnWindows {
        pids: vec![2398],
        classes: Vec::new(),
    };
    let front = hyprland.frontmost_window(&own).unwrap();
    assert_eq!(front.map(|w| w.wm_class), Some("firefox".into()));
}

#[test]
fn hyprland_dispatches_the_lua_form_and_falls_back_to_the_classic_one() {
    let dir = tempfile::tempdir().unwrap();
    let (fake, hyprland) = fake_hyprland(dir.path(), "ok");
    hyprland.focus_window("0x5581c8a4f310").unwrap();
    assert_eq!(
        fake.seen(),
        [r#"dispatch hl.dsp.focus({ window = "address:0x5581c8a4f310" })"#]
    );

    let dir = tempfile::tempdir().unwrap();
    let (fake, hyprland) = fake_hyprland(dir.path(), "Invalid dispatcher");
    hyprland.close_window("0x5581c8a4f310").unwrap();
    hyprland.focus_workspace("3").unwrap();
    assert_eq!(
        fake.seen(),
        [
            r#"dispatch hl.dsp.window.close({ window = "address:0x5581c8a4f310" })"#,
            "dispatch closewindow address:0x5581c8a4f310",
            r#"dispatch hl.dsp.focus({ workspace = "3" })"#,
            "dispatch workspace 3",
        ]
    );
}

#[test]
fn hyprland_with_nothing_focused_has_no_focused_window() {
    let dir = tempfile::tempdir().unwrap();
    let path = FakeSocket::hyprland_path(dir.path(), "sig");
    let _fake = FakeSocket::replaying(
        &path,
        Framing::Hyprland,
        vec![("-j/activewindow".into(), "{}".into())],
    );
    assert_eq!(Hyprland::new(path).focused_window().unwrap(), None);
}

#[test]
fn a_hyprland_that_is_not_there_is_an_error_not_a_hang() {
    let dir = tempfile::tempdir().unwrap();
    let hyprland = Hyprland::new(dir.path().join("hypr/none/.socket.sock"));
    assert!(hyprland.windows().is_err());
    assert!(!hyprland.ping());
}

fn niri(dir: &Path) -> (FakeSocket, Niri) {
    let path = dir.join("niri.wayland-1.sock");
    let fake = FakeSocket::replaying(
        &path,
        Framing::Niri,
        vec![
            ("\"Windows\"".into(), fixture("niri/windows.json")),
            ("\"Workspaces\"".into(), fixture("niri/workspaces.json")),
            (
                "\"FocusedWindow\"".into(),
                fixture("niri/focused_window.json"),
            ),
            ("\"Version\"".into(), r#"{"Ok":{"Version":"26.04"}}"#.into()),
            ("{\"Action\"".into(), r#"{"Ok":"Handled"}"#.into()),
        ],
    );
    (fake, Niri::new(path))
}

#[test]
fn niri_lists_windows_most_recently_focused_first() {
    let dir = tempfile::tempdir().unwrap();
    let (_fake, niri) = niri(dir.path());
    let windows = niri.windows().unwrap();
    assert_eq!(
        windows.iter().map(|w| w.id.as_str()).collect::<Vec<_>>(),
        ["7", "4", "3", "9"],
        "by focus timestamp, never-focused last"
    );
    assert_eq!(windows[0].pid, Some(2398));
    assert_eq!(windows[0].workspace.as_deref(), Some("1"));
    assert!(windows[0].focused);
    assert_eq!(windows[3].pid, None, "a portal window has no pid");
    assert_eq!(
        niri.focused_window().unwrap().map(|w| w.id),
        Some("7".into())
    );
    assert!(niri.ping());
}

#[test]
fn niri_workspaces_the_focused_one_is_active() {
    let dir = tempfile::tempdir().unwrap();
    let (_fake, niri) = niri(dir.path());
    let workspaces = niri.workspaces().unwrap();
    assert_eq!(
        workspaces
            .iter()
            .map(|w| (w.id.as_str(), w.name.as_str(), w.number))
            .collect::<Vec<_>>(),
        [
            ("1", "", Some(1)),
            ("2", "chat", Some(2)),
            ("5", "", Some(1))
        ]
    );
    assert_eq!(
        niri.active_workspace().unwrap().map(|w| w.id),
        Some("1".into())
    );
}

#[test]
fn niri_frontmost_skips_the_launcher() {
    let dir = tempfile::tempdir().unwrap();
    let (_fake, niri) = niri(dir.path());
    let own = OwnWindows {
        pids: Vec::new(),
        classes: vec!["FOOT".into()],
    };
    assert_eq!(
        niri.frontmost_window(&own).unwrap().map(|w| w.id),
        Some("4".into())
    );
}

#[test]
fn niri_actions_are_sent_as_niri_serialises_them() {
    let dir = tempfile::tempdir().unwrap();
    let (fake, niri) = niri(dir.path());
    niri.focus_window("7").unwrap();
    niri.close_window("3").unwrap();
    niri.focus_workspace("2").unwrap();
    assert!(niri.focus_window("0x55").is_err(), "not a niri id");
    let seen: Vec<serde_json::Value> = fake
        .seen()
        .iter()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(
        seen,
        [
            serde_json::json!({"Action": {"FocusWindow": {"id": 7}}}),
            serde_json::json!({"Action": {"CloseWindow": {"id": 3}}}),
            serde_json::json!({"Action": {"FocusWorkspace": {"reference": {"Id": 2}}}}),
        ]
    );
}

#[test]
fn a_niri_refusal_is_its_message() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("niri.sock");
    let _fake = FakeSocket::replaying(
        &path,
        Framing::Niri,
        vec![("".into(), r#"{"Err":"window not found"}"#.into())],
    );
    let err = Niri::new(path).focus_window("99").unwrap_err();
    assert_eq!(err.to_string(), "refused: window not found");
}

#[test]
fn the_provider_reaches_whichever_compositor_the_environment_names() {
    let dir = tempfile::tempdir().unwrap();
    let (_fake, _) = fake_hyprland(dir.path(), "ok");
    let runtime = dir.path().to_path_buf();
    let provider = Provider::detect_from(|name| match name {
        "HYPRLAND_INSTANCE_SIGNATURE" => Some("v0.50_1700000000".into()),
        "XDG_RUNTIME_DIR" => Some(runtime.clone().into_os_string()),
        _ => None,
    })
    .expect("Hyprland");
    assert_eq!(provider.id(), "hyprland");
    assert_eq!(provider.windows().unwrap().len(), 3);
    assert_eq!(provider.workspaces().unwrap().len(), 2);
}
