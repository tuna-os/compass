//! The `Application` half of the extension API.
//!
//! Ports `ExtApplicationService`
//! (`src/server/src/extension/api/application-service.hpp`): five methods over
//! the application database, which sits behind [`Apps`] here for the same
//! reason `AppService` is injected there.
//!
//! The database itself is `compass-core`'s [`crate::application_service`]-free
//! business — `AppIndex` already reads the desktop entries, and `compass-xdg`'s
//! `mimeapps` already resolves default openers. What this module owns is the
//! translation: which query each method makes, what it does when the query
//! comes back empty, and the exact shape of an `Application` on the wire.

use crate::tsapi::{self, Call};

/// The methods this serves, as they appear on the wire.
pub const METHODS: &[&str] = &[
    "Application/list",
    "Application/open",
    "Application/getDefault",
    "Application/showInFileBrowser",
    "Application/runInTerminal",
];

/// The error `getDefault` fails with, verbatim from the C++.
pub const NO_DEFAULT: &str = "No default application found";

/// One application, as `appToTsapi` builds it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Application {
    /// The desktop-entry id, e.g. `org.gnome.Nautilus.desktop`.
    pub id: String,
    /// `displayName()`.
    pub name: String,
    /// `iconUrl()`, stringified. Optional in the IDL but never omitted here,
    /// because the C++ assigns it unconditionally.
    pub icon: String,
    /// The path to the desktop entry.
    pub path: String,
}

/// `LaunchTerminalCommandOptions`, as `runInTerminal` fills it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TerminalOptions {
    /// Keep the terminal open after the command exits.
    pub hold: bool,
    /// Use this terminal rather than the preferred one.
    pub app_id: Option<String>,
    /// The window title to ask for.
    pub title: Option<String>,
    /// Where to run.
    pub working_directory: Option<String>,
}

/// The application database an extension can reach.
pub trait Apps {
    /// Every known application.
    fn list(&self) -> Vec<Application>;

    /// Every application that can open `target`, best first.
    fn openers(&self, target: &str) -> Vec<Application>;

    /// The preferred application for `target`.
    fn default_opener(&self, target: &str) -> Option<Application>;

    /// The application with this desktop-entry id.
    fn by_id(&self, id: &str) -> Option<Application>;

    /// Launches `app` with `target` as its sole argument.
    fn launch(&self, app: &Application, target: &str);

    /// Opens the file browser at `target`, selecting the entry when asked.
    fn show_in_file_browser(&self, target: &str, select: bool);

    /// Runs `cmdline` in a terminal; the bool is what the method returns.
    fn run_in_terminal(&self, cmdline: &[String], options: &TerminalOptions) -> bool;
}

/// Serves `Application` from one database.
#[derive(Debug)]
pub struct ApplicationService<A> {
    apps: A,
}

impl<A: Apps> ApplicationService<A> {
    /// Serves `apps`.
    pub const fn new(apps: A) -> Self {
        Self { apps }
    }

    /// The database this serves.
    pub const fn apps(&self) -> &A {
        &self.apps
    }

    /// Answers `call`, or `None` if it is not an `Application` call.
    #[must_use]
    pub fn handle(&self, call: &Call) -> Option<String> {
        let id = call.id?;
        if !METHODS.contains(&call.method.as_str()) {
            return None;
        }

        let string = |name: &str| {
            call.params
                .get(name)
                .filter(|value| !value.is_null())
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
        };

        Some(match call.method.as_str() {
            "Application/list" => {
                // `target ? findOpeners(target) : list()` -- an absent target
                // lists everything, a present one narrows to what can open it.
                let apps = match string("target") {
                    Some(target) => self.apps.openers(&target),
                    None => self.apps.list(),
                };
                let apps: Vec<serde_json::Value> = apps.iter().map(application).collect();
                tsapi::reply(id, serde_json::Value::Array(apps))
            }
            "Application/open" => {
                let target = string("target").unwrap_or_default();
                self.open(&target, string("appId").as_deref());
                tsapi::reply(id, serde_json::Value::Null)
            }
            "Application/getDefault" => {
                let target = string("target").unwrap_or_default();
                match self.apps.default_opener(&target) {
                    Some(app) => tsapi::reply(id, application(&app)),
                    None => tsapi::reply_error(id, NO_DEFAULT),
                }
            }
            "Application/showInFileBrowser" => {
                let target = string("target").unwrap_or_default();
                let select = call
                    .params
                    .get("select")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false);
                self.apps.show_in_file_browser(&target, select);
                tsapi::reply(id, serde_json::Value::Null)
            }
            _ => {
                let opts = call.params.get("opts");
                let field = |name: &str| {
                    opts.and_then(|opts| opts.get(name))
                        .filter(|value| !value.is_null())
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_owned)
                };
                let cmdline: Vec<String> = opts
                    .and_then(|opts| opts.get("cmdline"))
                    .and_then(serde_json::Value::as_array)
                    .map(|args| {
                        args.iter()
                            .map(|arg| arg.as_str().unwrap_or_default().to_owned())
                            .collect()
                    })
                    .unwrap_or_default();
                let options = TerminalOptions {
                    hold: opts
                        .and_then(|opts| opts.get("hold"))
                        .and_then(serde_json::Value::as_bool)
                        .unwrap_or(false),
                    app_id: field("appId"),
                    title: field("title"),
                    working_directory: field("workingDirectory"),
                };
                let ok = self.apps.run_in_terminal(&cmdline, &options);
                tsapi::reply(id, serde_json::Value::Bool(ok))
            }
        })
    }

    /// `open`'s two-step lookup.
    ///
    /// The `appId` branch returns *only if the id resolves*: the C++ `if (auto
    /// app = findById(...))` falls through when it does not, so an extension
    /// naming an application that is not installed still gets the file opened
    /// by the default handler rather than nothing happening.
    fn open(&self, target: &str, app_id: Option<&str>) {
        if let Some(app) = app_id.and_then(|id| self.apps.by_id(id)) {
            self.apps.launch(&app, target);
            return;
        }
        if let Some(opener) = self.apps.default_opener(target) {
            self.apps.launch(&opener, target);
        }
    }
}

impl<A: Apps> tsapi::Service for ApplicationService<A> {
    fn handle(&self, call: &Call) -> Option<String> {
        Self::handle(self, call)
    }
}

/// One `Application`, as the IDL declares it.
fn application(app: &Application) -> serde_json::Value {
    serde_json::json!({
        "id": app.id,
        "name": app.name,
        "icon": app.icon,
        "path": app.path,
    })
}
