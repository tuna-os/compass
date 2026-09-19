//! `WindowManagement/*`, read against
//! `src/server/src/extension/api/wm-service.hpp` and `figura/tsapi.fig`.

use std::cell::RefCell;

use compass_worker_host::application_service::Application;
use compass_worker_host::tsapi::Call;
use compass_worker_host::window_service::{
    BOUNDS_REFUSED, NO_ACTIVE_WINDOW, NO_ACTIVE_WORKSPACE, NO_SUCH_WINDOW, Rect, Screen, Size,
    Window, WindowService, Windows, Workspace,
};

#[derive(Default)]
struct Stub {
    windows: Vec<Window>,
    focused: Option<Window>,
    frontmost: Option<Application>,
    workspaces: Vec<Workspace>,
    active_workspace: Option<Workspace>,
    screens: Vec<Screen>,
    app_for_class: Option<Application>,
    bounds_ok: bool,
    focused_ids: RefCell<Vec<String>>,
    bounds_set: RefCell<Vec<(String, Rect)>>,
}

impl Windows for Stub {
    fn find_window(&self, id: &str) -> Option<Window> {
        self.windows.iter().find(|w| w.id == id).cloned()
    }
    fn focus(&self, window: &Window) {
        self.focused_ids.borrow_mut().push(window.id.clone());
    }
    fn focused_window(&self) -> Option<Window> {
        self.focused.clone()
    }
    fn frontmost_app(&self) -> Option<Application> {
        self.frontmost.clone()
    }
    fn list_windows(&self) -> Vec<Window> {
        self.windows.clone()
    }
    fn active_workspace(&self) -> Option<Workspace> {
        self.active_workspace.clone()
    }
    fn list_workspaces(&self) -> Vec<Workspace> {
        self.workspaces.clone()
    }
    fn list_screens(&self) -> Vec<Screen> {
        self.screens.clone()
    }
    fn app_for_class(&self, _wm_class: &str) -> Option<Application> {
        self.app_for_class.clone()
    }
    fn set_window_bounds(&self, window: &Window, bounds: Rect) -> bool {
        self.bounds_set
            .borrow_mut()
            .push((window.id.clone(), bounds));
        self.bounds_ok
    }
}

fn window(id: &str, workspace: Option<&str>) -> Window {
    Window {
        id: id.to_owned(),
        title: format!("{id} title"),
        workspace_id: workspace.map(str::to_owned),
        fullscreen: false,
        bounds: Some(Rect {
            x: 1,
            y: 2,
            width: 3,
            height: 4,
        }),
        wm_class: format!("{id}-class"),
    }
}

fn call(method: &str, params: serde_json::Value) -> Call {
    serde_json::from_value(serde_json::json!({
        "jsonrpc": "2.0", "id": 9, "method": method, "params": params,
    }))
    .expect("a well-formed call")
}

fn reply(
    service: &WindowService<Stub>,
    method: &str,
    params: serde_json::Value,
) -> serde_json::Value {
    let answer = service.handle(&call(method, params)).expect("answered");
    serde_json::from_str(&answer).expect("a JSON reply")
}

#[test]
fn focusing_an_unknown_window_is_false_rather_than_an_error() {
    // `if (!win) return Result<bool>::ok(false);` -- windows close between
    // being listed and being focused, and an extension checks the bool.
    let service = WindowService::new(Stub::default());
    let answer = reply(
        &service,
        "WindowManagement/focusWindow",
        serde_json::json!({"winId": "gone"}),
    );

    assert_eq!(answer["result"], serde_json::Value::Bool(false));
    assert!(answer.get("error").is_none(), "{answer}");
    assert!(service.windows().focused_ids.borrow().is_empty());
}

#[test]
fn focusing_a_known_window_focuses_it_and_reports_true() {
    let service = WindowService::new(Stub {
        windows: vec![window("w1", Some("1"))],
        ..Stub::default()
    });
    let answer = reply(
        &service,
        "WindowManagement/focusWindow",
        serde_json::json!({"winId": "w1"}),
    );

    assert_eq!(answer["result"], serde_json::Value::Bool(true));
    assert_eq!(*service.windows().focused_ids.borrow(), ["w1"]);
}

#[test]
fn a_window_carries_every_required_field_even_when_unset() {
    // `serializeWindow` leaves `workspaceId` and `bounds` alone when the window
    // has neither, and glaze writes the value-initialised "" and {0,0,0,0}.
    let service = WindowService::new(Stub {
        focused: Some(Window {
            id: "w".to_owned(),
            title: "T".to_owned(),
            ..Window::default()
        }),
        ..Stub::default()
    });
    let answer = reply(
        &service,
        "WindowManagement/getActiveWindow",
        serde_json::json!({}),
    );

    assert_eq!(
        answer["result"],
        serde_json::json!({
            "id": "w",
            "title": "T",
            "workspaceId": "",
            "active": true,
            "fullscreen": false,
            "bounds": {"x": 0, "y": 0, "width": 0, "height": 0},
        })
    );
}

#[test]
fn the_active_window_carries_its_application_when_the_class_resolves() {
    let app = Application {
        id: "org.gnome.Nautilus.desktop".to_owned(),
        name: "Files".to_owned(),
        icon: "file:///i.png".to_owned(),
        path: "/usr/share/applications/nautilus.desktop".to_owned(),
    };
    let service = WindowService::new(Stub {
        focused: Some(window("w1", Some("1"))),
        app_for_class: Some(app),
        ..Stub::default()
    });
    let answer = reply(
        &service,
        "WindowManagement/getActiveWindow",
        serde_json::json!({}),
    );

    assert_eq!(answer["result"]["app"]["name"], "Files");
    assert_eq!(answer["result"]["active"], serde_json::Value::Bool(true));
    assert_eq!(answer["result"]["bounds"]["width"], 3);
}

#[test]
fn with_no_focused_window_the_frontmost_app_stands_in_for_one() {
    // "Platforms without window-level focus (macOS) still know the frontmost
    // app" -- the fallback answers a `Window` built from the application, so it
    // has no workspace and no bounds.
    let service = WindowService::new(Stub {
        focused: None,
        frontmost: Some(Application {
            id: "com.apple.Safari".to_owned(),
            name: "Safari".to_owned(),
            icon: String::new(),
            path: "/Applications/Safari.app".to_owned(),
        }),
        ..Stub::default()
    });
    let answer = reply(
        &service,
        "WindowManagement/getActiveWindow",
        serde_json::json!({}),
    );

    assert_eq!(answer["result"]["id"], "com.apple.Safari");
    assert_eq!(answer["result"]["title"], "Safari");
    assert_eq!(answer["result"]["active"], serde_json::Value::Bool(true));
    assert_eq!(answer["result"]["app"]["id"], "com.apple.Safari");
    assert_eq!(answer["result"]["bounds"]["width"], 0);
}

#[test]
fn with_neither_the_active_window_call_fails_with_the_cpp_message() {
    let service = WindowService::new(Stub::default());
    let answer = reply(
        &service,
        "WindowManagement/getActiveWindow",
        serde_json::json!({}),
    );

    assert_eq!(answer["error"], NO_ACTIVE_WINDOW);
    assert_eq!(NO_ACTIVE_WINDOW, "No active window");
}

#[test]
fn listing_windows_marks_the_focused_one_and_filters_by_workspace() {
    let service = WindowService::new(Stub {
        windows: vec![
            window("w1", Some("1")),
            window("w2", Some("2")),
            window("w3", Some("1")),
        ],
        focused: Some(window("w3", Some("1"))),
        ..Stub::default()
    });

    let all = reply(
        &service,
        "WindowManagement/getWindows",
        serde_json::json!({}),
    );
    let all = all["result"].as_array().cloned().unwrap_or_default();
    assert_eq!(all.len(), 3);
    assert_eq!(all[0]["active"], serde_json::Value::Bool(false));
    assert_eq!(all[2]["active"], serde_json::Value::Bool(true));

    let one = reply(
        &service,
        "WindowManagement/getWindows",
        serde_json::json!({"workspaceId": "1"}),
    );
    let ids: Vec<&str> = one["result"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["w1", "w3"]);
}

#[test]
fn a_window_with_no_workspace_matches_only_a_request_for_the_empty_one() {
    // `win->workspace().value_or("") != *workspaceId` -- the `value_or("")` is
    // what makes an unassigned window answer to "".
    let service = WindowService::new(Stub {
        windows: vec![window("floating", None), window("w1", Some("1"))],
        ..Stub::default()
    });

    let empty = reply(
        &service,
        "WindowManagement/getWindows",
        serde_json::json!({"workspaceId": ""}),
    );
    let ids: Vec<&str> = empty["result"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["floating"]);

    let one = reply(
        &service,
        "WindowManagement/getWindows",
        serde_json::json!({"workspaceId": "1"}),
    );
    assert_eq!(one["result"].as_array().map(Vec::len), Some(1));
    assert_eq!(one["result"][0]["id"], "w1");
}

#[test]
fn the_active_workspace_is_active_by_construction_or_the_call_fails() {
    let service = WindowService::new(Stub::default());
    let answer = reply(
        &service,
        "WindowManagement/getActiveWorkspace",
        serde_json::json!({}),
    );
    assert_eq!(answer["error"], NO_ACTIVE_WORKSPACE);
    assert_eq!(NO_ACTIVE_WORKSPACE, "No active workspace");

    let service = WindowService::new(Stub {
        active_workspace: Some(Workspace {
            id: "2".to_owned(),
            name: "Two".to_owned(),
            fullscreen: true,
            monitor: Some("DP-1".to_owned()),
        }),
        ..Stub::default()
    });
    let answer = reply(
        &service,
        "WindowManagement/getActiveWorkspace",
        serde_json::json!({}),
    );
    assert_eq!(
        answer["result"],
        serde_json::json!({
            "id": "2", "name": "Two", "active": true, "fullscreen": true, "monitor": "DP-1",
        })
    );
}

#[test]
fn listing_workspaces_marks_the_active_one_and_omits_an_absent_monitor() {
    let service = WindowService::new(Stub {
        workspaces: vec![
            Workspace {
                id: "1".to_owned(),
                name: "One".to_owned(),
                fullscreen: false,
                monitor: None,
            },
            Workspace {
                id: "2".to_owned(),
                name: "Two".to_owned(),
                fullscreen: false,
                monitor: Some("DP-1".to_owned()),
            },
        ],
        active_workspace: Some(Workspace {
            id: "2".to_owned(),
            ..Workspace::default()
        }),
        ..Stub::default()
    });
    let answer = reply(
        &service,
        "WindowManagement/getWorkspaces",
        serde_json::json!({}),
    );

    assert_eq!(
        answer["result"][0]["active"],
        serde_json::Value::Bool(false)
    );
    assert!(
        answer["result"][0].get("monitor").is_none(),
        "{}",
        answer["result"][0]
    );
    assert_eq!(answer["result"][1]["active"], serde_json::Value::Bool(true));
    assert_eq!(answer["result"][1]["monitor"], "DP-1");
}

#[test]
fn a_screen_maps_its_seven_fields_and_omits_an_absent_serial() {
    let service = WindowService::new(Stub {
        screens: vec![
            Screen {
                name: "DP-1".to_owned(),
                model: "U2720Q".to_owned(),
                make: "Dell".to_owned(),
                serial: Some("ABC123".to_owned()),
                bounds: Rect {
                    x: 0,
                    y: 0,
                    width: 3840,
                    height: 2160,
                },
                physical_resolution: Size {
                    width: 3840,
                    height: 2160,
                },
                active: true,
            },
            Screen {
                name: "HDMI-1".to_owned(),
                ..Screen::default()
            },
        ],
        ..Stub::default()
    });
    let answer = reply(
        &service,
        "WindowManagement/getScreens",
        serde_json::json!({}),
    );

    assert_eq!(
        answer["result"][0],
        serde_json::json!({
            "name": "DP-1",
            "model": "U2720Q",
            "make": "Dell",
            "serial": "ABC123",
            "bounds": {"x": 0, "y": 0, "width": 3840, "height": 2160},
            "physicalResolution": {"width": 3840, "height": 2160},
            "active": true,
        })
    );
    assert!(
        answer["result"][1].get("serial").is_none(),
        "{}",
        answer["result"][1]
    );
}

#[test]
fn setting_bounds_fails_distinctly_for_a_missing_window_and_a_refusal() {
    // Two different messages, because they are two different problems for the
    // extension: one is a stale id, the other a compositor that said no.
    let service = WindowService::new(Stub::default());
    let answer = reply(
        &service,
        "WindowManagement/setWindowBounds",
        serde_json::json!({"winId": "gone", "bounds": {"x": 0, "y": 0, "width": 1, "height": 1}}),
    );
    assert_eq!(answer["error"], NO_SUCH_WINDOW);
    assert_eq!(NO_SUCH_WINDOW, "No window with the given id");

    let service = WindowService::new(Stub {
        windows: vec![window("w1", Some("1"))],
        bounds_ok: false,
        ..Stub::default()
    });
    let answer = reply(
        &service,
        "WindowManagement/setWindowBounds",
        serde_json::json!({"winId": "w1", "bounds": {"x": 5, "y": 6, "width": 7, "height": 8}}),
    );
    assert_eq!(answer["error"], BOUNDS_REFUSED);
    assert_eq!(BOUNDS_REFUSED, "Failed to set window bounds");
    assert_eq!(
        *service.windows().bounds_set.borrow(),
        [(
            "w1".to_owned(),
            Rect {
                x: 5,
                y: 6,
                width: 7,
                height: 8
            }
        )]
    );
}

#[test]
fn setting_bounds_that_the_provider_accepts_is_a_void_reply() {
    let service = WindowService::new(Stub {
        windows: vec![window("w1", Some("1"))],
        bounds_ok: true,
        ..Stub::default()
    });
    let answer = reply(
        &service,
        "WindowManagement/setWindowBounds",
        serde_json::json!({"winId": "w1", "bounds": {"x": 5, "y": 6, "width": 7, "height": 8}}),
    );

    assert_eq!(answer["result"], serde_json::Value::Null);
    assert!(answer.get("error").is_none(), "{answer}");
}

#[test]
fn another_services_call_is_not_answered_here() {
    let service = WindowService::new(Stub::default());
    assert!(
        service
            .handle(&call("Storage/get", serde_json::json!({})))
            .is_none()
    );
}

#[test]
fn an_event_is_not_answered_and_focuses_nothing() {
    let service = WindowService::new(Stub {
        windows: vec![window("w1", Some("1"))],
        ..Stub::default()
    });
    let event: Call = serde_json::from_value(serde_json::json!({
        "jsonrpc": "2.0",
        "method": "WindowManagement/focusWindow",
        "params": {"winId": "w1"},
    }))
    .unwrap();

    assert!(service.handle(&event).is_none());
    assert!(service.windows().focused_ids.borrow().is_empty());
}
