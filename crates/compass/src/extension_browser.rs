//! The browser bridge an extension reaches through `BrowserExtension/*`.
//!
//! The C++ `BrowserExtensionService` is fed by the browser's native messaging
//! host over the C++ IPC. ADR-0008 took browser control out of the port, so
//! no browser ever registers with this engine: it answers as the C++ does
//! with no browser connected — `getTabs` is an empty list, `focusTab` asks
//! nothing of anyone and succeeds — and `environment.canAccess(
//! BrowserExtension)` is `false`, which is the C++'s own capability test
//! (`!browsers().empty()`).

use compass_worker_host::browser_service::{Browser, Tab};

/// [`Browser`] with no browser connected.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoBrowsers;

impl Browser for NoBrowsers {
    fn tabs(&self) -> Vec<Tab> {
        Vec::new()
    }

    fn focus_tab(&self, browser_id: &str, tab_id: i64) {
        // `emit tabActionRequested(...)` with no browser to receive it.
        tracing::debug!(browser_id, tab_id, "focusTab with no browser connected");
    }
}
