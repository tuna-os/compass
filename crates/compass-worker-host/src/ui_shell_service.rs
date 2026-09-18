//! The rest of `UI`: toasts, HUD, navigation, search text, selected text and
//! desktop notifications.
//!
//! Ports the non-rendering half of `ExtUIService`
//! (`src/server/src/extension/api/ui-service.hpp`). [`crate::ui_service`] keeps
//! `UI/render` and the navigation stack; this is everything that asks the shell
//! around the view to do something, behind a [`Shell`] trait.
//!
//! `UI/confirmAlert` is deliberately still unimplemented: it shows a dialog and
//! answers whenever the *user* does, which is not a call into a backend but a
//! suspended reply, and the host has no way to hold one open yet. A `Shell`
//! method returning a bool would have to block the whole session on a person.

use crate::tsapi::{self, Call};

/// The methods this serves, as they appear on the wire.
pub const METHODS: &[&str] = &[
    "UI/showToast",
    "UI/updateToast",
    "UI/hideToast",
    "UI/showHud",
    "UI/closeMainWindow",
    "UI/popToRoot",
    "UI/pushView",
    "UI/popView",
    "UI/setSearchText",
    "UI/getSelectedText",
    "UI/sendDesktopNotification",
];

/// The toast kinds the shell knows.
///
/// The IDL's `Error` is the shell's `Danger`; the rest keep their names, and an
/// unknown one is `Success`, which is the C++ `default:` arm.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ToastStyle {
    /// `ToastStyle::Success`, and the fallback.
    #[default]
    Success,
    /// `ToastStyle::Info`.
    Info,
    /// `ToastStyle::Warning`.
    Warning,
    /// The IDL's `Error`.
    Danger,
    /// `ToastStyle::Dynamic`.
    Dynamic,
}

impl ToastStyle {
    /// The C++ `mapToastStyle`.
    #[must_use]
    pub fn from_wire(name: &str) -> Self {
        match name {
            "Info" => Self::Info,
            "Warning" => Self::Warning,
            "Error" => Self::Danger,
            "Dynamic" => Self::Dynamic,
            _ => Self::Success,
        }
    }
}

/// What closing the window should do to the root view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PopToRoot {
    /// `PopToRootType::Default`, and the fallback.
    #[default]
    Default,
    /// Pop now.
    Immediate,
    /// Leave the stack where it is.
    Suspended,
}

impl PopToRoot {
    /// The C++ `mapPopToRoot`.
    #[must_use]
    pub fn from_wire(name: &str) -> Self {
        match name {
            "Immediate" => Self::Immediate,
            "Suspended" => Self::Suspended,
            _ => Self::Default,
        }
    }
}

/// How loudly a desktop notification asks for attention.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Urgency {
    /// `Urgency::Low`.
    Low,
    /// `Urgency::Normal`, and the fallback.
    #[default]
    Normal,
    /// `Urgency::High`.
    High,
}

impl Urgency {
    /// The C++ `mapNotificationUrgency`.
    #[must_use]
    pub fn from_wire(name: &str) -> Self {
        match name {
            "Low" => Self::Low,
            "High" => Self::High,
            _ => Self::Normal,
        }
    }
}

/// `NavigationController::CloseWindowOptions`, plus the delay the caller asks
/// for. `closeMainWindow` passes 50ms where `showHud` passes none.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CloseWindow {
    /// What to do with the view stack.
    pub pop_to_root: PopToRoot,
    /// Whether to empty the root search bar.
    pub clear_root_search: bool,
    /// How long to wait first, in milliseconds.
    pub delay_ms: u64,
}

/// The delay `closeMainWindow` asks for, verbatim from the C++ `50ms`.
pub const CLOSE_MAIN_WINDOW_DELAY_MS: u64 = 50;

/// A desktop notification, as `sendDesktopNotification` builds one.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Notification {
    /// The bold line.
    pub title: String,
    /// The body.
    pub body: String,
    /// The icon the extension sent, untouched. The C++ renders it to a
    /// temporary PNG and passes the path; that rendering is the shell's job,
    /// so the source travels instead of a path.
    pub icon: Option<serde_json::Value>,
    /// How loudly to ask.
    pub urgency: Urgency,
}

/// What the running command is called, for the view it pushes.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CommandInfo {
    /// `ExtensionCommand::name`.
    pub name: String,
    /// `ExtensionCommand::iconUrl`, stringified.
    pub icon: String,
}

/// The shell around an extension's view.
pub trait Shell {
    /// `ToastService::setToast`.
    fn set_toast(&self, title: &str, style: ToastStyle, message: &str);

    /// `ToastService::clear`.
    fn clear_toast(&self);

    /// `NavigationController::closeWindow`.
    fn close_window(&self, options: CloseWindow);

    /// `NavigationController::showHud`.
    fn show_hud(&self, text: &str);

    /// `NavigationController::popToRoot`.
    fn pop_to_root(&self, clear_search: bool);

    /// `pushView` plus the navigation title and icon it sets from the command.
    fn push_view(&self, command: &CommandInfo);

    /// `NavigationController::popCurrentView`.
    fn pop_view(&self);

    /// `NavigationController::setSearchText`.
    fn set_search_text(&self, text: &str);

    /// `AbstractSelectionService::selectedText`; the error is the service's own.
    fn selected_text(&self) -> Result<String, String>;

    /// `DesktopNotificationClient::send`.
    fn send_notification(&self, notification: &Notification);
}

/// Serves the shell half of `UI` for one running command.
#[derive(Debug)]
pub struct UiShellService<S> {
    shell: S,
    command: CommandInfo,
}

impl<S: Shell> UiShellService<S> {
    /// Serves `shell` on behalf of `command`.
    pub const fn new(shell: S, command: CommandInfo) -> Self {
        Self { shell, command }
    }

    /// The shell this serves.
    pub const fn shell(&self) -> &S {
        &self.shell
    }

    /// Answers `call`, or `None` if it is not one of ours.
    #[must_use]
    #[allow(clippy::too_many_lines)]
    pub fn handle(&self, call: &Call) -> Option<String> {
        let id = call.id?;
        if !METHODS.contains(&call.method.as_str()) {
            return None;
        }

        let string = |name: &str| {
            call.params
                .get(name)
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
        };
        let flag = |name: &str| {
            call.params
                .get(name)
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false)
        };

        Some(match call.method.as_str() {
            "UI/showToast" => {
                // `setToast(title, style, message)` -- the IDL's order is
                // (id, title, message, style), and `id` is not used at all.
                self.shell.set_toast(
                    string("title"),
                    ToastStyle::from_wire(string("style")),
                    string("message"),
                );
                tsapi::reply(id, serde_json::Value::Null)
            }
            // `Void::Future updateToast(...) override { return Void::ok(); }`
            // -- the C++ body is empty. Reproduced rather than implemented:
            // a Rust host that updated the toast would show text the C++ host
            // never shows, and an extension cannot tell the difference between
            // the two from the reply. See PARITY.md.
            "UI/updateToast" => tsapi::reply(id, serde_json::Value::Null),
            // `hideToast` ignores its id and clears whatever is showing.
            "UI/hideToast" => {
                self.shell.clear_toast();
                tsapi::reply(id, serde_json::Value::Null)
            }
            "UI/showHud" => {
                // Close first, then show: the HUD outlives the window.
                self.shell.close_window(CloseWindow {
                    pop_to_root: PopToRoot::from_wire(string("popToRoot")),
                    // The IDL spells this one `clear_root`, not `clearRoot`.
                    clear_root_search: flag("clear_root"),
                    delay_ms: 0,
                });
                self.shell.show_hud(string("text"));
                tsapi::reply(id, serde_json::Value::Null)
            }
            "UI/closeMainWindow" => {
                self.shell.close_window(CloseWindow {
                    pop_to_root: PopToRoot::from_wire(string("popToRoot")),
                    clear_root_search: flag("clearRoot"),
                    delay_ms: CLOSE_MAIN_WINDOW_DELAY_MS,
                });
                tsapi::reply(id, serde_json::Value::Null)
            }
            "UI/popToRoot" => {
                self.shell.pop_to_root(flag("clearSearchBar"));
                tsapi::reply(id, serde_json::Value::Null)
            }
            "UI/pushView" => {
                self.shell.push_view(&self.command);
                tsapi::reply(id, serde_json::Value::Null)
            }
            "UI/popView" => {
                self.shell.pop_view();
                tsapi::reply(id, serde_json::Value::Null)
            }
            "UI/setSearchText" => {
                self.shell.set_search_text(string("text"));
                tsapi::reply(id, serde_json::Value::Null)
            }
            "UI/getSelectedText" => match self.shell.selected_text() {
                Ok(text) => tsapi::reply(id, serde_json::Value::String(text)),
                // `if (!result) return std::unexpected(result.error())` -- the
                // selection service's own message, not one invented here.
                Err(error) => tsapi::reply_error(id, &error),
            },
            _ => {
                let data = call.params.get("data");
                let field = |name: &str| {
                    data.and_then(|data| data.get(name))
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or_default()
                };
                self.shell.send_notification(&Notification {
                    title: field("title").to_owned(),
                    body: field("body").to_owned(),
                    icon: data
                        .and_then(|data| data.get("icon"))
                        .filter(|icon| !icon.is_null())
                        .cloned(),
                    urgency: Urgency::from_wire(field("urgency")),
                });
                tsapi::reply(id, serde_json::Value::Null)
            }
        })
    }
}

impl<S: Shell> tsapi::Service for UiShellService<S> {
    fn handle(&self, call: &Call) -> Option<String> {
        Self::handle(self, call)
    }
}

/// The `UI/viewPushed` event, which `pushView` raises.
///
/// Separate from the reply because it *is* separate in the C++:
/// `QTimer::singleShot(0, ...)` emits it on the next turn of the event loop,
/// after the reply has gone out. A host that sent it first would have the
/// extension see its view appear before the call that asked for it returned.
#[must_use]
pub fn view_pushed() -> String {
    tsapi::event("UI/viewPushed", serde_json::json!({}))
}

/// The `UI/viewPoped` event — the IDL's spelling, not a typo here.
///
/// The C++ raises it from `handleViewPoped` only when more than one view is on
/// the stack, so popping the last one is silent; whoever owns the stack keeps
/// that condition, since this module does not hold it.
#[must_use]
pub fn view_popped() -> String {
    tsapi::event("UI/viewPoped", serde_json::json!({}))
}
