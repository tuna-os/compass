//! The HUD: the pill the C++ shows for a moment after an action that hides
//! the launcher ("Copied to clipboard", "Quit Files", "Wallpaper set").
//!
//! Ports `HudBridge` (`ui/windows/hud-bridge.*`) and `HudWindow.qml`. The C++
//! shows it only where a window can appear without taking the focus from the
//! application the launcher hid back to — on Linux, a `wlr-layer-shell`
//! surface with no keyboard interactivity (`Environment::isHudSupported`).
//! Everywhere else (GNOME's `xdg_toplevel`) `NavigationController::showHud`
//! only closes the window, and so does [`HudState::show`]'s caller.
//!
//! The state here is plain data: which surface is the HUD's, what it says,
//! and when it goes. A new message moves the deadline, as `m_timer.start()`
//! restarts the C++'s timer, and the launcher ticks only while one is set.
//! The surface is asked for by `crate::surface::open_hud`.

use std::time::Instant;

use iced::window;

/// How long the HUD stays, as `HudBridge`'s timer.
pub const DURATION: std::time::Duration = std::time::Duration::from_millis(1500);

/// The layer surface's namespace (`LayerShell.Window.scope`), which
/// compositors match rules on.
pub const NAMESPACE: &str = "vicinae-hud";

/// The HUD surface's size. The pill is centred in it and the rest is
/// transparent and takes no input; the width is the C++'s widest pill (a
/// 270 px text column, the 16 px icon and its 5 px gap, 30 px of padding).
pub const SURFACE_SIZE: (u32, u32) = (336, 48);

/// The widest the text runs before it is elided, as `Layout.maximumWidth`.
pub const TEXT_MAX_WIDTH: f32 = 270.0;

/// The icon the C++'s generic copy action shows (`BuiltinIcon::CopyClipboard`).
pub const COPY_ICON: &str = "copy-clipboard";

/// What the HUD says: one line and an optional icon — a builtin icon's name,
/// or an emoji drawn as text (`ImageURL::emoji`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hud {
    /// The line of text.
    pub text: String,
    /// A builtin icon name or an emoji.
    pub icon: Option<String>,
}

impl Hud {
    /// A HUD with no icon.
    #[must_use]
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            icon: None,
        }
    }

    /// The same HUD with `icon`.
    #[must_use]
    pub fn with_icon(mut self, icon: impl Into<String>) -> Self {
        self.icon = Some(icon.into());
        self
    }

    /// "Copied to clipboard" with the clipboard icon, as `CopyToClipboardAction`.
    #[must_use]
    pub fn copied() -> Self {
        Self::new("Copied to clipboard").with_icon(COPY_ICON)
    }

    /// Whether the icon is one of the builtin set rather than an emoji.
    #[must_use]
    pub fn icon_is_builtin(&self) -> bool {
        self.icon
            .as_deref()
            .is_some_and(compass_core::builtin_icon::is_builtin)
    }
}

/// What showing a HUD asks of the window system.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// Nothing: this presentation has no HUD.
    Unsupported,
    /// Open the HUD surface under this id.
    Open(window::Id),
    /// The surface is already up; only its text changed.
    Update,
}

/// The HUD's state in the launcher.
#[derive(Debug, Default)]
pub struct HudState {
    supported: bool,
    window: Option<window::Id>,
    content: Option<Hud>,
    deadline: Option<Instant>,
}

impl HudState {
    /// A HUD that shows only when `supported`.
    #[must_use]
    pub fn new(supported: bool) -> Self {
        Self {
            supported,
            ..Self::default()
        }
    }

    /// Whether this presentation shows a HUD.
    #[must_use]
    pub fn supported(&self) -> bool {
        self.supported
    }

    /// Turns the HUD on or off, for a presentation decided after boot and
    /// for tests.
    pub fn set_supported(&mut self, supported: bool) {
        self.supported = supported;
    }

    /// What the HUD shows now, if it is up.
    #[must_use]
    pub fn content(&self) -> Option<&Hud> {
        self.content.as_ref()
    }

    /// Whether the HUD is up and its timer running, which is when the
    /// launcher subscribes to ticks.
    #[must_use]
    pub fn counting(&self) -> bool {
        self.deadline.is_some()
    }

    /// The HUD's surface, while it is up.
    #[must_use]
    pub fn window_id(&self) -> Option<window::Id> {
        self.window
    }

    /// Whether `id` is the HUD's surface.
    #[must_use]
    pub fn owns(&self, id: window::Id) -> bool {
        self.window == Some(id)
    }

    /// Shows `hud` at `now`: a surface is opened the first time, the text of
    /// the one up is replaced after that, and the timer restarts either way.
    pub fn show(&mut self, hud: Hud, now: Instant) -> Step {
        if !self.supported {
            return Step::Unsupported;
        }
        self.content = Some(hud);
        self.deadline = Some(now + DURATION);
        match self.window {
            Some(_) => Step::Update,
            None => {
                let id = window::Id::unique();
                self.window = Some(id);
                Step::Open(id)
            }
        }
    }

    /// The time is `now`: the surface to close once the timer has run out.
    pub fn tick(&mut self, now: Instant) -> Option<window::Id> {
        if self.deadline.is_none_or(|deadline| now < deadline) {
            return None;
        }
        self.deadline = None;
        self.content = None;
        self.window.take()
    }

    /// A surface closed; `true` when it was the HUD's.
    pub fn closed(&mut self, id: window::Id) -> bool {
        if self.window == Some(id) {
            self.window = None;
            self.content = None;
            self.deadline = None;
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unsupported_presentation_shows_nothing() {
        let mut hud = HudState::new(false);
        assert_eq!(hud.show(Hud::copied(), Instant::now()), Step::Unsupported);
        assert!(hud.content().is_none());
        assert!(!hud.counting());
    }

    #[test]
    fn a_second_message_reuses_the_surface_and_restarts_the_timer() {
        let start = Instant::now();
        let mut hud = HudState::new(true);
        let Step::Open(id) = hud.show(Hud::new("Quit Files"), start) else {
            panic!("the first HUD opens a surface");
        };
        assert!(hud.owns(id));
        let later = start + DURATION / 2;
        assert_eq!(hud.show(Hud::copied(), later), Step::Update);
        assert_eq!(hud.tick(start + DURATION), None, "the timer restarted");
        assert_eq!(hud.content(), Some(&Hud::copied()));
        assert_eq!(hud.tick(later + DURATION), Some(id));
        assert!(hud.content().is_none());
        assert!(!hud.owns(id));
        assert!(!hud.counting());
    }

    #[test]
    fn a_surface_closed_under_it_is_forgotten() {
        let start = Instant::now();
        let mut hud = HudState::new(true);
        let Step::Open(id) = hud.show(Hud::copied(), start) else {
            panic!("opens");
        };
        assert!(!hud.closed(window::Id::unique()));
        assert!(hud.closed(id));
        assert_eq!(hud.tick(start + DURATION), None);
        assert!(matches!(hud.show(Hud::copied(), start), Step::Open(_)));
    }

    #[test]
    fn the_copy_hud_is_the_cpps() {
        let hud = Hud::copied();
        assert_eq!(hud.text, "Copied to clipboard");
        assert!(hud.icon_is_builtin());
        assert!(
            !Hud::new("Clipboard cleared")
                .with_icon("🤫")
                .icon_is_builtin()
        );
    }
}
