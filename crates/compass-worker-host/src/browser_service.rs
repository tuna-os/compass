//! The `BrowserExtension` half of the extension API.
//!
//! Ports `ExtBrowserExtensionService`
//! (`src/server/src/extension/api/ext-browser-extension-service.hpp`): two
//! methods over whatever is talking to the browser extension.

use crate::tsapi::{self, Call};

/// The methods this serves, as they appear on the wire.
pub const METHODS: &[&str] = &["BrowserExtension/getTabs", "BrowserExtension/focusTab"];

/// One open tab. Mirrors `tsapi::BrowserTab`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Tab {
    /// The browser's own tab id.
    pub id: i64,
    /// The page title, when the browser reports one; optional in the IDL.
    pub title: Option<String>,
    /// The page URL.
    pub url: String,
    /// Whether it is the active tab in its window.
    pub active: bool,
    /// Which browser it belongs to.
    pub browser_id: String,
}

/// The browser bridge an extension can reach.
pub trait Browser {
    /// `BrowserExtensionService::tabs`.
    fn tabs(&self) -> Vec<Tab>;

    /// `BrowserExtensionService::focusTab`.
    fn focus_tab(&self, browser_id: &str, tab_id: i64);
}

/// Serves `BrowserExtension` from one bridge.
#[derive(Debug)]
pub struct BrowserService<B> {
    browser: B,
}

impl<B: Browser> BrowserService<B> {
    /// Serves `browser`.
    pub const fn new(browser: B) -> Self {
        Self { browser }
    }

    /// The bridge this serves.
    pub const fn browser(&self) -> &B {
        &self.browser
    }

    /// Answers `call`, or `None` if it is not a `BrowserExtension` call.
    #[must_use]
    pub fn handle(&self, call: &Call) -> Option<String> {
        let id = call.id?;
        if !METHODS.contains(&call.method.as_str()) {
            return None;
        }

        Some(if call.method == "BrowserExtension/getTabs" {
            let tabs: Vec<serde_json::Value> = self.browser.tabs().iter().map(tab).collect();
            tsapi::reply(id, serde_json::Value::Array(tabs))
        } else {
            let browser_id = call
                .params
                .get("browserId")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            // `tabId` is an `int` in the IDL, and the C++ takes it as one; a
            // caller that sends a string or a float gets zero, the same value
            // its `int32_t` would be left with.
            let tab_id = call
                .params
                .get("tabId")
                .and_then(serde_json::Value::as_i64)
                .unwrap_or(0);
            self.browser.focus_tab(browser_id, tab_id);
            tsapi::reply(id, serde_json::Value::Null)
        })
    }
}

impl<B: Browser> tsapi::Service for BrowserService<B> {
    fn handle(&self, call: &Call) -> Option<String> {
        Self::handle(self, call)
    }
}

/// One `BrowserTab`; `title` is optional and omitted when the browser has none.
fn tab(tab: &Tab) -> serde_json::Value {
    let mut out = serde_json::json!({
        "id": tab.id,
        "url": tab.url,
        "active": tab.active,
        "browserId": tab.browser_id,
    });
    if let Some(title) = &tab.title {
        out["title"] = serde_json::Value::String(title.clone());
    }
    out
}
