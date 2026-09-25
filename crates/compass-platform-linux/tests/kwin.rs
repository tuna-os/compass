//! The KWin provider against a fake `org.kde.KWin` on a private bus.
//!
//! The fake answers `org.kde.kwin.Scripting` as KWin does — `loadScript`
//! gives a number and serves `/Scripting/Script<n>`, whose `run` reads the
//! file it was given — and, having no JavaScript engine, replays what KWin
//! would do running the script: for the tracker, `callDBus` `add` for each
//! normal window and `activated` for the active one; for a one-shot, the
//! change to its target and the tracker call KWin's signal would produce.
//! It also serves `org.kde.KWin.VirtualDesktopManager` and, on its own name,
//! kglobalaccel's KWin component. Real KWin is the VM tier.

mod support;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use compass_platform_linux::compositor::kwin::{
    self, Kwin, TRACKER_INTERFACE, TRACKER_JS, TRACKER_PLUGIN, TRACKER_SERVICE,
};
use compass_platform_linux::compositor::{OwnWindows, Provider};
use support::bus;

#[derive(Debug, Clone)]
struct FakeWindow {
    internal_id: String,
    class: String,
    caption: String,
    pid: i32,
    desktop: String,
    fullscreen: bool,
    normal: bool,
}

fn fake_window(n: u32, class: &str, caption: &str, pid: i32, desktop: &str) -> FakeWindow {
    FakeWindow {
        internal_id: format!("{{0000000{n}-0000-0000-0000-000000000000}}"),
        class: class.into(),
        caption: caption.into(),
        pid,
        desktop: desktop.into(),
        fullscreen: false,
        normal: true,
    }
}

#[derive(Debug, Default)]
struct KwinState {
    windows: Vec<FakeWindow>,
    active: Option<String>,
    desktops: Vec<(u32, String, String)>,
    current: String,
    /// id -> (path, plugin)
    loaded: Vec<(i32, String, String)>,
    unloaded: Vec<String>,
    /// (plugin, source read at `run`)
    ran: Vec<(String, String)>,
    overview: u32,
}

type Shared = Arc<Mutex<KwinState>>;

struct Scripting {
    state: Shared,
}

#[zbus::interface(name = "org.kde.kwin.Scripting")]
impl Scripting {
    #[zbus(name = "loadScript")]
    async fn load_script(
        &self,
        path: String,
        plugin: String,
        #[zbus(object_server)] server: &zbus::ObjectServer,
    ) -> i32 {
        let id = {
            let mut state = self.state.lock().unwrap();
            let id = i32::try_from(state.loaded.len()).unwrap();
            state.loaded.push((id, path, plugin));
            id
        };
        server
            .at(
                format!("/Scripting/Script{id}"),
                Script {
                    id,
                    state: Arc::clone(&self.state),
                },
            )
            .await
            .unwrap();
        id
    }

    #[zbus(name = "unloadScript")]
    fn unload_script(&self, plugin: String) -> bool {
        let mut state = self.state.lock().unwrap();
        let known = state.loaded.iter().any(|(_, _, p)| *p == plugin);
        state.unloaded.push(plugin);
        known
    }
}

struct Script {
    id: i32,
    state: Shared,
}

/// A string literal the one-shot script declares as `const <name> = …;`.
fn literal(source: &str, name: &str) -> String {
    let start = source
        .find(&format!("const {name} = "))
        .expect("the declaration")
        + name.len()
        + 9;
    let end = start + source[start..].find(";\n").expect("its end");
    serde_json::from_str(&source[start..end]).expect("a string literal")
}

type Add = (String, String, String, String, i32, String, i32);

fn add_args(w: &FakeWindow) -> Add {
    (
        w.internal_id.clone(),
        w.class.clone(),
        w.class.to_lowercase(),
        w.caption.clone(),
        w.pid,
        w.desktop.clone(),
        i32::from(w.fullscreen),
    )
}

async fn tell(
    connection: &zbus::Connection,
    method: &str,
    body: &(impl serde::Serialize + zbus::zvariant::DynamicType),
) {
    connection
        .call_method(
            Some(TRACKER_SERVICE),
            "/",
            Some(TRACKER_INTERFACE),
            method,
            body,
        )
        .await
        .unwrap_or_else(|err| panic!("callDBus {method}: {err}"));
}

#[zbus::interface(name = "org.kde.kwin.Script")]
impl Script {
    #[zbus(name = "run")]
    async fn run(&self, #[zbus(connection)] connection: &zbus::Connection) {
        let (path, plugin) = {
            let state = self.state.lock().unwrap();
            let (_, path, plugin) = state.loaded[usize::try_from(self.id).unwrap()].clone();
            (path, plugin)
        };
        // KWin opens the file here, not at loadScript.
        let source = std::fs::read_to_string(&path).expect("the script file is still there");
        self.state
            .lock()
            .unwrap()
            .ran
            .push((plugin.clone(), source.clone()));

        if plugin == TRACKER_PLUGIN {
            let (windows, active) = {
                let state = self.state.lock().unwrap();
                (state.windows.clone(), state.active.clone())
            };
            for window in windows.iter().filter(|w| w.normal) {
                tell(connection, "add", &add_args(window)).await;
            }
            tell(connection, "activated", &(active.unwrap_or_default(),)).await;
            return;
        }

        let target = literal(&source, "target");
        let action = literal(&source, "action");
        let effect = {
            let mut state = self.state.lock().unwrap();
            let Some(index) = state.windows.iter().position(|w| w.internal_id == target) else {
                return;
            };
            match action.as_str() {
                "focus" => {
                    state.active = Some(target.clone());
                    ("activated", None)
                }
                "close" => {
                    state.windows.remove(index);
                    ("remove", None)
                }
                "fullscreen" => {
                    state.windows[index].fullscreen = !state.windows[index].fullscreen;
                    ("add", Some(add_args(&state.windows[index])))
                }
                other => panic!("unknown action {other}"),
            }
        };
        match effect {
            (method, None) => tell(connection, method, &(target,)).await,
            (method, Some(args)) => tell(connection, method, &args).await,
        }
    }
}

struct Desktops {
    state: Shared,
}

#[zbus::interface(name = "org.kde.KWin.VirtualDesktopManager")]
impl Desktops {
    #[zbus(property, name = "desktops")]
    fn desktops(&self) -> Vec<(u32, String, String)> {
        self.state.lock().unwrap().desktops.clone()
    }

    #[zbus(property, name = "current")]
    fn current(&self) -> String {
        self.state.lock().unwrap().current.clone()
    }

    #[zbus(property, name = "current")]
    fn set_current(&mut self, id: String) -> zbus::fdo::Result<()> {
        let mut state = self.state.lock().unwrap();
        if !state.desktops.iter().any(|(_, known, _)| *known == id) {
            return Err(zbus::fdo::Error::InvalidArgs(format!("no desktop {id}")));
        }
        state.current = id;
        Ok(())
    }
}

struct Component {
    state: Shared,
}

#[zbus::interface(name = "org.kde.kglobalaccel.Component")]
impl Component {
    #[zbus(name = "invokeShortcut")]
    fn invoke_shortcut(&self, name: String) {
        assert_eq!(name, kwin::OVERVIEW_SHORTCUT);
        self.state.lock().unwrap().overview += 1;
    }
}

/// Owns `org.kde.KWin` (and kglobalaccel) on `address` until dropped.
async fn fake_kwin(address: &str, state: &Shared) -> zbus::Connection {
    zbus::connection::Builder::address(address)
        .unwrap()
        .name(kwin::KWIN_SERVICE)
        .unwrap()
        .name(kwin::KGLOBALACCEL_SERVICE)
        .unwrap()
        .serve_at(
            kwin::SCRIPTING_PATH,
            Scripting {
                state: Arc::clone(state),
            },
        )
        .unwrap()
        .serve_at(
            kwin::DESKTOPS_PATH,
            Desktops {
                state: Arc::clone(state),
            },
        )
        .unwrap()
        .serve_at(
            kwin::KWIN_COMPONENT_PATH,
            Component {
                state: Arc::clone(state),
            },
        )
        .unwrap()
        .build()
        .await
        .unwrap()
}

async fn engine(address: &str) -> zbus::Connection {
    zbus::connection::Builder::address(address)
        .unwrap()
        .build()
        .await
        .unwrap()
}

fn plasma() -> Shared {
    let mut firefox = fake_window(1, "firefox", "Mozilla Firefox", 100, "d-one");
    firefox.fullscreen = false;
    let mut panel = fake_window(9, "plasmashell", "Panel", 50, "");
    panel.normal = false;
    Arc::new(Mutex::new(KwinState {
        windows: vec![
            firefox,
            panel,
            fake_window(2, "org.kde.konsole", "~ : bash", 200, "d-two"),
            fake_window(3, "org.kde.dolphin", "Home", 300, ""),
        ],
        active: Some("{00000002-0000-0000-0000-000000000000}".into()),
        desktops: vec![
            (0, "d-one".into(), "Desktop 1".into()),
            (1, "d-two".into(), "Work".into()),
            (2, "d-three".into(), String::new()),
        ],
        current: "d-two".into(),
        ..KwinState::default()
    }))
}

/// Runs `work` on a blocking thread, as the engine calls the provider.
async fn blocking<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> T {
    tokio::task::spawn_blocking(work).await.unwrap()
}

async fn eventually(what: &str, mut done: impl FnMut() -> bool) {
    for _ in 0..200 {
        if done() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("never: {what}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_tracker_lists_kwins_normal_windows_and_which_is_active() {
    let Some(bus) = bus::start_or_skip("the_tracker_lists_kwins_normal_windows") else {
        return;
    };
    let state = plasma();
    let _kwin = fake_kwin(bus.address(), &state).await;
    let provider = Kwin::start(engine(bus.address()).await).await.unwrap();

    {
        let state = state.lock().unwrap();
        assert_eq!(
            state.unloaded.first().map(String::as_str),
            Some(TRACKER_PLUGIN),
            "a tracker left by an earlier run is unloaded first"
        );
        assert_eq!(state.loaded[0].2, TRACKER_PLUGIN);
        assert_eq!(state.ran[0].1, TRACKER_JS, "KWin read the tracker script");
    }

    let windows = provider.windows();
    assert_eq!(
        windows
            .iter()
            .map(|w| (w.id.as_str(), w.title.as_str(), w.wm_class.as_str(), w.pid))
            .collect::<Vec<_>>(),
        [
            ("1", "Mozilla Firefox", "firefox", Some(100)),
            ("2", "~ : bash", "org.kde.konsole", Some(200)),
            ("3", "Home", "org.kde.dolphin", Some(300)),
        ],
        "the panel is not a normal window"
    );
    assert_eq!(windows[0].workspace.as_deref(), Some("d-one"));
    assert_eq!(windows[2].workspace, None, "on all desktops");
    assert_eq!(
        provider.focused_window().map(|w| w.id),
        Some("2".to_owned())
    );

    let provider = Provider::Kwin(provider);
    assert_eq!(provider.id(), "kde");
    assert_eq!(provider.socket(), None);
    assert!(blocking(move || provider.ping()).await);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn focus_close_and_fullscreen_are_one_shot_scripts_unloaded_after() {
    let Some(bus) = bus::start_or_skip("focus_close_and_fullscreen_are_one_shot_scripts") else {
        return;
    };
    let state = plasma();
    let _kwin = fake_kwin(bus.address(), &state).await;
    let provider = Provider::Kwin(Kwin::start(engine(bus.address()).await).await.unwrap());

    let p = provider.clone();
    blocking(move || p.focus_window("1")).await.unwrap();
    assert_eq!(
        state.lock().unwrap().active.as_deref(),
        Some("{00000001-0000-0000-0000-000000000000}")
    );
    let p = provider.clone();
    let focused = blocking(move || p.focused_window()).await.unwrap();
    assert_eq!(focused.map(|w| w.id), Some("1".to_owned()));

    let p = provider.clone();
    blocking(move || p.toggle_fullscreen("2")).await.unwrap();
    assert!(state.lock().unwrap().windows[2].fullscreen);
    let p = provider.clone();
    let windows = blocking(move || p.windows()).await.unwrap();
    assert!(windows[1].fullscreen, "the tracker heard fullScreenChanged");

    let p = provider.clone();
    blocking(move || p.close_window("3")).await.unwrap();
    let p = provider.clone();
    let windows = blocking(move || p.windows()).await.unwrap();
    assert_eq!(
        windows.iter().map(|w| w.id.as_str()).collect::<Vec<_>>(),
        ["1", "2"]
    );

    {
        let state = state.lock().unwrap();
        let one_shots: Vec<&str> = state.ran[1..].iter().map(|(p, _)| p.as_str()).collect();
        assert_eq!(one_shots.len(), 3);
        for plugin in one_shots {
            assert!(plugin.starts_with("vicinae-"), "{plugin}");
            assert!(
                state.unloaded.iter().any(|u| u == plugin),
                "{plugin} was left loaded"
            );
        }
        assert!(
            state.ran[1]
                .1
                .contains("{00000001-0000-0000-0000-000000000000}")
        );
    }

    let p = provider.clone();
    let refused = blocking(move || p.focus_window("3")).await;
    assert!(refused.is_err(), "a closed window is not focused");
    let p = provider.clone();
    assert!(blocking(move || p.toggle_floating("1")).await.is_err());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn virtual_desktops_are_the_workspaces_and_the_overview_is_kwins_shortcut() {
    let Some(bus) = bus::start_or_skip("virtual_desktops_are_the_workspaces") else {
        return;
    };
    let state = plasma();
    let _kwin = fake_kwin(bus.address(), &state).await;
    let provider = Provider::Kwin(Kwin::start(engine(bus.address()).await).await.unwrap());

    let caps = provider.capabilities();
    assert!(caps.workspaces && caps.fullscreen && caps.overview);
    assert!(!caps.floating);

    let p = provider.clone();
    let workspaces = blocking(move || p.workspaces()).await.unwrap();
    assert_eq!(
        workspaces
            .iter()
            .map(|w| (w.id.as_str(), w.name.as_str(), w.number))
            .collect::<Vec<_>>(),
        [
            ("d-one", "Desktop 1", Some(1)),
            ("d-two", "Work", Some(2)),
            ("d-three", "d-three", Some(3)),
        ]
    );
    let p = provider.clone();
    let active = blocking(move || p.active_workspace()).await.unwrap();
    assert_eq!(active.map(|w| w.id), Some("d-two".to_owned()));

    // The launcher (pid 200 here) is skipped; firefox is on another desktop.
    let own = OwnWindows {
        pids: vec![200],
        classes: Vec::new(),
    };
    let p = provider.clone();
    let o = own.clone();
    blocking(move || p.focus_window("1")).await.unwrap();
    let p = provider.clone();
    blocking(move || p.focus_window("2")).await.unwrap();
    let p = provider.clone();
    let frontmost = blocking(move || p.frontmost_window(&o)).await.unwrap();
    assert_eq!(
        frontmost, None,
        "the only other window used is on another desktop"
    );

    let p = provider.clone();
    blocking(move || p.focus_workspace("d-one")).await.unwrap();
    assert_eq!(state.lock().unwrap().current, "d-one");
    let p = provider.clone();
    let frontmost = blocking(move || p.frontmost_window(&own)).await.unwrap();
    assert_eq!(frontmost.map(|w| w.id), Some("1".to_owned()));

    let p = provider.clone();
    assert!(
        blocking(move || p.focus_workspace("nowhere"))
            .await
            .is_err(),
        "KWin's refusal is passed on"
    );

    let p = provider.clone();
    blocking(move || p.toggle_overview()).await.unwrap();
    assert_eq!(state.lock().unwrap().overview, 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_kwin_restart_forgets_its_windows_and_reloads_the_tracker() {
    let Some(bus) = bus::start_or_skip("a_kwin_restart_forgets_its_windows") else {
        return;
    };
    let provider = Kwin::start(engine(bus.address()).await).await.unwrap();
    assert!(provider.windows().is_empty(), "no KWin yet is no windows");

    let state = plasma();
    let first = fake_kwin(bus.address(), &state).await;
    eventually("the tracker is loaded when KWin appears", || {
        provider.windows().len() == 3
    })
    .await;

    drop(first);
    eventually("KWin's windows are forgotten when it goes", || {
        provider.windows().is_empty()
    })
    .await;

    let state = Arc::new(Mutex::new(KwinState {
        windows: vec![fake_window(7, "kate", "notes.txt", 700, "")],
        ..KwinState::default()
    }));
    let _second = fake_kwin(bus.address(), &state).await;
    eventually("the tracker is reloaded in the new KWin", || {
        provider.windows().len() == 1
    })
    .await;
    assert_eq!(
        provider.windows()[0].id,
        "4",
        "handles are never reused across KWin sessions"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_second_tracker_is_refused_the_name_and_stop_gives_it_up() {
    let Some(bus) = bus::start_or_skip("a_second_tracker_is_refused_the_name") else {
        return;
    };
    let state = plasma();
    let _kwin = fake_kwin(bus.address(), &state).await;
    let first = Kwin::start(engine(bus.address()).await).await.unwrap();
    assert!(matches!(
        Kwin::start(engine(bus.address()).await).await,
        Err(kwin::StartError::NameTaken)
    ));

    first.stop().await;
    assert_eq!(
        state.lock().unwrap().unloaded.last().map(String::as_str),
        Some(TRACKER_PLUGIN)
    );
    assert!(Kwin::start(engine(bus.address()).await).await.is_ok());
}
