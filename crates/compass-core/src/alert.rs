//! The confirmation dialog: what it says by default, and what answer each way
//! out of it gives.
//!
//! A port of `AlertWidget` and `AlertModel` (`src/server/src/ui/alert/`),
//! minus the widget. What matters here is not how the dialog is drawn but
//! which of `true` and `false` the caller is handed, because on the other end
//! of that boolean is usually something irreversible — clearing clipboard
//! history, revoking an OAuth token, running a script, suspending the machine.
//!
//! # Every way out that is not the confirm button answers `false`
//!
//! There are four: the confirm button, the cancel button, navigating to
//! another view, and a *second* alert arriving while this one is open. Only
//! the first answers `true`. `AlertModel` gets this right by routing the last
//! three through `triggerCancel`, and this port keeps that shape rather than
//! reimplementing each — a fifth exit added later should have to opt in to
//! `true`, not out of it.

/// A colour named by role rather than value, as `ColorLike` holding a
/// `SemanticColor`.
///
/// The theme that turns one of these into an actual colour is not ported, so
/// this carries the name the C++ names and stops there. Resolving it here
/// would mean inventing a palette.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SemanticColor(&'static str);

impl SemanticColor {
    /// `SemanticColor::Red`, the destructive colour.
    pub const RED: Self = Self("Red");
    /// `SemanticColor::Foreground`, ordinary text.
    pub const FOREGROUND: Self = Self("Foreground");

    /// The role's name.
    #[must_use]
    pub const fn name(self) -> &'static str {
        self.0
    }
}

/// The icon an alert shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlertIcon {
    /// The image's source: a built-in icon's name, or something else.
    pub source: String,
    /// Whether `source` names a built-in icon.
    ///
    /// This drives the badge the model sets, and it is also what decides
    /// whether the icon gets tinted red — the C++ only fills built-ins.
    pub builtin: bool,
    /// The fill applied to it, where one is applied.
    pub fill: Option<SemanticColor>,
}

impl AlertIcon {
    /// The default icon: the built-in warning triangle, filled red.
    #[must_use]
    pub fn warning() -> Self {
        Self {
            source: "warning".to_string(),
            builtin: true,
            fill: Some(SemanticColor::RED),
        }
    }

    /// An icon from an extension's payload.
    ///
    /// `ExtUIService::confirmAlert` fills a built-in red and leaves anything
    /// else — a file, a URL — untouched, because tinting a photograph red is
    /// not a highlight, it is damage.
    #[must_use]
    pub fn from_payload(source: impl Into<String>, builtin: bool) -> Self {
        Self {
            source: source.into(),
            builtin,
            fill: if builtin {
                Some(SemanticColor::RED)
            } else {
                None
            },
        }
    }
}

/// The default title, from `AlertWidget`'s member initialiser.
pub const DEFAULT_TITLE: &str = "Are you sure?";
/// The default message.
pub const DEFAULT_MESSAGE: &str = "This action cannot be undone";
/// The default confirm button's text.
pub const DEFAULT_CONFIRM_TEXT: &str = "Confirm";
/// The default cancel button's text.
pub const DEFAULT_CANCEL_TEXT: &str = "Cancel";

/// One alert, as the widget holds it before the model shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Alert {
    /// The heading.
    pub title: String,
    /// The body.
    pub message: String,
    /// The confirm button's text.
    pub confirm_text: String,
    /// The cancel button's text.
    pub cancel_text: String,
    /// The confirm button's colour.
    pub confirm_color: SemanticColor,
    /// The cancel button's colour.
    pub cancel_color: SemanticColor,
    /// The icon, where there is one.
    pub icon: Option<AlertIcon>,
}

impl Default for Alert {
    /// Every default `AlertWidget` starts with, in the same order.
    fn default() -> Self {
        Self {
            title: DEFAULT_TITLE.to_string(),
            message: DEFAULT_MESSAGE.to_string(),
            confirm_text: DEFAULT_CONFIRM_TEXT.to_string(),
            cancel_text: DEFAULT_CANCEL_TEXT.to_string(),
            confirm_color: SemanticColor::RED,
            cancel_color: SemanticColor::FOREGROUND,
            icon: Some(AlertIcon::warning()),
        }
    }
}

impl Alert {
    /// An alert with the defaults.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the heading.
    #[must_use]
    pub fn with_title(mut self, title: impl Into<String>) -> Self {
        self.title = title.into();
        self
    }

    /// Set the body.
    #[must_use]
    pub fn with_message(mut self, message: impl Into<String>) -> Self {
        self.message = message.into();
        self
    }

    /// Set the confirm button's text and colour, which `setConfirmText` takes
    /// together because a button that says "Delete" in the ordinary colour is
    /// a different button.
    #[must_use]
    pub fn with_confirm(mut self, text: impl Into<String>, color: SemanticColor) -> Self {
        self.confirm_text = text.into();
        self.confirm_color = color;
        self
    }

    /// Set the cancel button's text and colour.
    #[must_use]
    pub fn with_cancel(mut self, text: impl Into<String>, color: SemanticColor) -> Self {
        self.cancel_text = text.into();
        self.cancel_color = color;
        self
    }

    /// Set, or clear, the icon.
    #[must_use]
    pub fn with_icon(mut self, icon: Option<AlertIcon>) -> Self {
        self.icon = icon;
        self
    }
}

/// How an alert ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// The confirm button. The only `true`.
    Confirmed,
    /// The cancel button.
    Cancelled,
    /// The view changed under it.
    Dismissed,
    /// Another alert replaced it.
    Replaced,
}

impl Outcome {
    /// The boolean the caller is handed.
    #[must_use]
    pub const fn confirmed(self) -> bool {
        matches!(self, Self::Confirmed)
    }
}

/// What the model tells the caller when an alert ends.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolution {
    /// Which alert ended.
    pub alert: Alert,
    /// How it ended.
    pub outcome: Outcome,
}

/// The alert model: at most one alert, and what happens to it.
///
/// `AlertModel` is a singleton fed by `NavigationController`; this is the same
/// state machine with the signals turned into return values, so a caller can
/// see the resolution rather than having to be connected to it.
#[derive(Debug, Default)]
pub struct AlertModel {
    /// The alert on screen, if any.
    current: Option<Alert>,
}

impl AlertModel {
    /// A model with nothing showing.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether an alert is on screen.
    #[must_use]
    pub fn is_visible(&self) -> bool {
        self.current.is_some()
    }

    /// The alert on screen.
    #[must_use]
    pub fn current(&self) -> Option<&Alert> {
        self.current.as_ref()
    }

    /// Show `alert`, cancelling whatever was showing.
    ///
    /// The returned resolution is the *previous* alert's, and it is
    /// [`Outcome::Replaced`] — which the TypeScript API documents: "Calling
    /// this function when another alert is currently pending will result in
    /// the pending alert to be automatically canceled". A caller that drops
    /// this return value leaves the first extension's promise unresolved
    /// forever.
    pub fn show(&mut self, alert: Alert) -> Option<Resolution> {
        let replaced = self.current.take().map(|alert| Resolution {
            alert,
            outcome: Outcome::Replaced,
        });
        self.current = Some(alert);
        replaced
    }

    /// The confirm button.
    pub fn confirm(&mut self) -> Option<Resolution> {
        self.finish(Outcome::Confirmed)
    }

    /// The cancel button.
    pub fn cancel(&mut self) -> Option<Resolution> {
        self.finish(Outcome::Cancelled)
    }

    /// The current view changed, which `AlertModel`'s constructor wires to
    /// `dismiss`.
    pub fn view_changed(&mut self) -> Option<Resolution> {
        self.finish(Outcome::Dismissed)
    }

    /// End the current alert, if there is one.
    ///
    /// Taking the alert out *before* reporting is what makes a second
    /// confirm a no-op rather than a second answer: the C++ nulls `m_widget`
    /// before it calls `triggerConfirm` for the same reason.
    fn finish(&mut self, outcome: Outcome) -> Option<Resolution> {
        self.current
            .take()
            .map(|alert| Resolution { alert, outcome })
    }
}
