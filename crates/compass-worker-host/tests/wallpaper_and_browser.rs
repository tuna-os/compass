//! `Wallpaper/set` and `BrowserExtension/*`, read against
//! `src/server/src/extension/api/wallpaper-service.hpp`,
//! `.../ext-browser-extension-service.hpp` and `figura/tsapi.fig`.

use std::cell::RefCell;

use compass_worker_host::browser_service::{Browser, BrowserService, Tab};
use compass_worker_host::tsapi::Call;
use compass_worker_host::wallpaper_service::{Fit, Request, Wallpaper, WallpaperService};

#[derive(Default)]
struct WallpaperStub {
    error: Option<String>,
    asked: RefCell<Vec<Request>>,
}

impl Wallpaper for WallpaperStub {
    fn set(&self, request: &Request) -> Result<(), String> {
        self.asked.borrow_mut().push(request.clone());
        self.error.clone().map_or(Ok(()), Err)
    }
}

#[derive(Default)]
struct BrowserStub {
    tabs: Vec<Tab>,
    focused: RefCell<Vec<(String, i64)>>,
}

impl Browser for BrowserStub {
    fn tabs(&self) -> Vec<Tab> {
        self.tabs.clone()
    }
    fn focus_tab(&self, browser_id: &str, tab_id: i64) {
        self.focused
            .borrow_mut()
            .push((browser_id.to_owned(), tab_id));
    }
}

fn call(method: &str, params: serde_json::Value) -> Call {
    serde_json::from_value(serde_json::json!({
        "jsonrpc": "2.0", "id": 4, "method": method, "params": params,
    }))
    .expect("a well-formed call")
}

fn parse(answer: &str) -> serde_json::Value {
    serde_json::from_str(answer).expect("a JSON reply")
}

#[test]
fn every_fit_the_idl_declares_maps_and_an_absent_one_is_cover() {
    // `options.fit.transform(mapFit).value_or(WallpaperFit::Cover)`
    for (name, fit) in [
        ("Cover", Fit::Cover),
        ("Contain", Fit::Contain),
        ("Stretch", Fit::Stretch),
        ("Center", Fit::Center),
        ("Tile", Fit::Tile),
    ] {
        let service = WallpaperService::new(WallpaperStub::default());
        service
            .handle(&call(
                "Wallpaper/set",
                serde_json::json!({"path": "/w.png", "options": {"fit": name}}),
            ))
            .expect("answered");
        assert_eq!(service.wallpaper().asked.borrow()[0].fit, fit, "{name}");
    }

    let service = WallpaperService::new(WallpaperStub::default());
    service
        .handle(&call(
            "Wallpaper/set",
            serde_json::json!({"path": "/w.png", "options": {}}),
        ))
        .expect("answered");
    assert_eq!(service.wallpaper().asked.borrow()[0].fit, Fit::Cover);
    assert_eq!(Fit::default(), Fit::Cover);
}

#[test]
fn a_fit_the_idl_does_not_declare_falls_back_to_cover() {
    // `transform(mapFit)` cannot be reached with a name glaze would not parse,
    // so there is no C++ behaviour to copy for one; falling back to the same
    // default keeps a malformed call from failing a wallpaper that would work.
    let service = WallpaperService::new(WallpaperStub::default());
    service
        .handle(&call(
            "Wallpaper/set",
            serde_json::json!({"path": "/w.png", "options": {"fit": "Parallax"}}),
        ))
        .expect("answered");

    assert_eq!(service.wallpaper().asked.borrow()[0].fit, Fit::Cover);
}

#[test]
fn the_path_and_screen_reach_the_backend() {
    let service = WallpaperService::new(WallpaperStub::default());
    let answer = parse(
        &service
            .handle(&call(
                "Wallpaper/set",
                serde_json::json!({"path": "/pictures/a.jpg", "options": {"screen": "DP-1"}}),
            ))
            .expect("answered"),
    );

    assert_eq!(answer["result"], serde_json::Value::Null);
    assert_eq!(
        service.wallpaper().asked.borrow()[0],
        Request {
            path: "/pictures/a.jpg".to_owned(),
            screen: Some("DP-1".to_owned()),
            fit: Fit::Cover,
        }
    );
}

#[test]
fn a_backend_that_refuses_rejects_the_extensions_promise_with_its_own_message() {
    // `return m_wallpaper.setWallpaper(req);` -- the manager's future is
    // returned directly, so its error is what the extension sees.
    let service = WallpaperService::new(WallpaperStub {
        error: Some("no wallpaper backend for this session".to_owned()),
        ..WallpaperStub::default()
    });
    let answer = parse(
        &service
            .handle(&call(
                "Wallpaper/set",
                serde_json::json!({"path": "/w.png", "options": {}}),
            ))
            .expect("answered"),
    );

    assert_eq!(answer["error"], "no wallpaper backend for this session");
}

#[test]
fn a_tab_carries_its_five_fields_and_omits_an_absent_title() {
    let service = BrowserService::new(BrowserStub {
        tabs: vec![
            Tab {
                id: 12,
                title: Some("Compass".to_owned()),
                url: "https://example.org".to_owned(),
                active: true,
                browser_id: "firefox".to_owned(),
            },
            Tab {
                id: 13,
                title: None,
                url: "about:blank".to_owned(),
                active: false,
                browser_id: "firefox".to_owned(),
            },
        ],
        ..BrowserStub::default()
    });
    let answer = parse(
        &service
            .handle(&call("BrowserExtension/getTabs", serde_json::json!({})))
            .expect("answered"),
    );

    assert_eq!(
        answer["result"][0],
        serde_json::json!({
            "id": 12,
            "title": "Compass",
            "url": "https://example.org",
            "active": true,
            "browserId": "firefox",
        })
    );
    assert!(
        answer["result"][1].get("title").is_none(),
        "{}",
        answer["result"][1]
    );
}

#[test]
fn focusing_a_tab_passes_the_browser_and_the_id_and_replies_null() {
    let service = BrowserService::new(BrowserStub::default());
    let answer = parse(
        &service
            .handle(&call(
                "BrowserExtension/focusTab",
                serde_json::json!({"browserId": "chrome", "tabId": 42}),
            ))
            .expect("answered"),
    );

    assert_eq!(answer["result"], serde_json::Value::Null);
    assert_eq!(
        *service.browser().focused.borrow(),
        [("chrome".to_owned(), 42)]
    );
}

#[test]
fn neither_service_answers_the_others_methods() {
    let wallpaper = WallpaperService::new(WallpaperStub::default());
    assert!(
        wallpaper
            .handle(&call("BrowserExtension/getTabs", serde_json::json!({})))
            .is_none()
    );
    assert!(wallpaper.wallpaper().asked.borrow().is_empty());

    let browser = BrowserService::new(BrowserStub::default());
    assert!(
        browser
            .handle(&call(
                "Wallpaper/set",
                serde_json::json!({"path": "/w.png"})
            ))
            .is_none()
    );
    assert!(browser.browser().focused.borrow().is_empty());
}

#[test]
fn an_event_is_not_answered_by_either() {
    let event = |method: &str| -> Call {
        serde_json::from_value(serde_json::json!({
            "jsonrpc": "2.0", "method": method, "params": {"path": "/w.png"},
        }))
        .unwrap()
    };

    let wallpaper = WallpaperService::new(WallpaperStub::default());
    assert!(wallpaper.handle(&event("Wallpaper/set")).is_none());
    assert!(wallpaper.wallpaper().asked.borrow().is_empty());

    let browser = BrowserService::new(BrowserStub::default());
    assert!(
        browser
            .handle(&event("BrowserExtension/focusTab"))
            .is_none()
    );
    assert!(browser.browser().focused.borrow().is_empty());
}
