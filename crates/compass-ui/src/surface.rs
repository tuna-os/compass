//! Which kind of surface the launcher window is: an `xdg_toplevel` (GNOME,
//! and every compositor without `wlr-layer-shell`) or a layer surface (the
//! wlroots family, `PLAN.md` Phase 5 Track B).
//!
//! `LauncherApp` opens and closes one window and does not care which. The
//! difference is confined to `open`: under `iced::daemon` a window is opened
//! with `window::open`, while `iced_layershell`'s runtime ignores that action
//! and instead takes a `NewLayerShell` request, which reaches it as a
//! [`Message::Layer`] converted by the `TryFrom` impl below — so the app's own
//! `update` never sees one. Closing is the same in both: `iced_layershell`
//! maps `window::close` to removing the surface and reports it through
//! `window::close_events`, exactly as `iced_winit` does.
//!
//! The choice is made once, by the binary, before the event loop starts; this
//! crate does not probe the compositor (ADR-0013: the shared crates name what
//! a platform can do, the binary picks).

use std::sync::OnceLock;

use iced::Task;
use iced::window;

use crate::message::Message;

/// How the launcher is presented.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Presentation {
    /// A plain `xdg_toplevel`, placed by the compositor.
    #[default]
    Toplevel,
    /// A `zwlr_layer_shell_v1` surface on the `top` layer, centred, taking
    /// the keyboard.
    LayerShell,
}

static PRESENTATION: OnceLock<Presentation> = OnceLock::new();

/// The presentation this process runs with.
#[must_use]
pub fn presentation() -> Presentation {
    PRESENTATION.get().copied().unwrap_or_default()
}

/// Fixes the presentation. Only the first call counts: switching surface kind
/// under a running event loop is not something either runtime supports.
pub(crate) fn set_presentation(presentation: Presentation) {
    let _ = PRESENTATION.set(presentation);
}

/// A request to `iced_layershell`'s runtime. Uninhabited off Linux, where
/// there is no layer shell to ask.
#[cfg(target_os = "linux")]
pub type LayerRequest = iced_layershell::actions::LayerShellCustomActionWithId;

/// A request to `iced_layershell`'s runtime. Uninhabited off Linux, where
/// there is no layer shell to ask.
#[cfg(not(target_os = "linux"))]
#[derive(Debug, Clone)]
pub enum LayerRequest {}

/// Opens the launcher window as this process presents it, returning its id
/// and the task that reports [`Message::Opened`] (a toplevel) or asks for the
/// surface (a layer surface, reported by [`opened_events`] instead).
pub(crate) fn open(settings: window::Settings) -> (window::Id, Task<Message>) {
    match presentation() {
        Presentation::Toplevel => {
            let (id, opened) = window::open(settings);
            (id, opened.map(Message::Opened))
        }
        #[cfg(target_os = "linux")]
        Presentation::LayerShell => {
            // No `Opened` here: the surface does not exist until the
            // compositor configures it, and the app focuses the search field
            // on `Opened` — a focus aimed at a window with no widget tree yet
            // is dropped, and the launcher comes up deaf. `opened_events`
            // reports it once `iced_layershell` has actually built it.
            let id = window::Id::unique();
            let request = layer::new_surface(id, &settings);
            (id, Task::done(Message::Layer(request)))
        }
        #[cfg(not(target_os = "linux"))]
        Presentation::LayerShell => {
            let (id, opened) = window::open(settings);
            (id, opened.map(Message::Opened))
        }
    }
}

/// `Opened` for layer surfaces, once they exist. Nothing under `iced::daemon`,
/// where `window::open`'s own task reports it.
pub(crate) fn opened_events() -> iced::Subscription<Message> {
    match presentation() {
        Presentation::LayerShell => window::open_events().map(Message::Opened),
        Presentation::Toplevel => iced::Subscription::none(),
    }
}

#[cfg(target_os = "linux")]
pub(crate) mod layer {
    use iced::window;
    use iced_layershell::actions::{LayerShellCustomAction, LayerShellCustomActionWithId};
    use iced_layershell::reexport::{
        Anchor, KeyboardInteractivity, Layer, NewLayerShellSettings, OutputOption,
    };

    use crate::message::Message;

    /// The layer surface's namespace, which compositors match rules on
    /// (`layer_effects "vicinae" …` in Hyprland, `layer-rule` in niri).
    pub const NAMESPACE: &str = "vicinae";

    /// The launcher as a layer surface: the window's own size, anchored to
    /// nothing so the compositor centres it, and taking the keyboard
    /// exclusively, which is what a launcher summoned by a hotkey needs —
    /// `OnDemand` would leave the keys with whatever had them until the user
    /// clicks. The `top` layer and exclusive keyboard are the C++ launcher's
    /// defaults (`LayerShellConfig`); its config keys that change them are not
    /// ported yet (PARITY.md, wlroots).
    pub fn settings(settings: &window::Settings) -> NewLayerShellSettings {
        NewLayerShellSettings {
            size: Some((
                settings.size.width.round() as u32,
                settings.size.height.round() as u32,
            )),
            layer: Layer::Top,
            anchor: Anchor::empty(),
            exclusive_zone: None,
            margin: None,
            keyboard_interactivity: KeyboardInteractivity::Exclusive,
            output_option: OutputOption::Active,
            events_transparent: false,
            namespace: Some(NAMESPACE.to_owned()),
        }
    }

    pub fn new_surface(
        id: window::Id,
        settings: &window::Settings,
    ) -> LayerShellCustomActionWithId {
        LayerShellCustomActionWithId::new(
            None,
            LayerShellCustomAction::NewLayerShell {
                settings: self::settings(settings),
                id,
            },
        )
    }

    impl TryFrom<Message> for LayerShellCustomActionWithId {
        type Error = Message;

        fn try_from(message: Message) -> Result<Self, Message> {
            match message {
                Message::Layer(request) => Ok(request),
                other => Err(other),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_presentation_is_a_toplevel() {
        // Nothing in the test process sets it, and GNOME must never get
        // anything else by default.
        assert_eq!(Presentation::default(), Presentation::Toplevel);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn the_layer_surface_is_centred_on_top_and_takes_the_keyboard() {
        use iced_layershell::reexport::{Anchor, KeyboardInteractivity, Layer};
        let settings = layer::settings(&window::Settings {
            size: iced::Size::new(768.0, 520.0),
            ..window::Settings::default()
        });
        assert_eq!(settings.size, Some((768, 520)));
        assert_eq!(settings.anchor, Anchor::empty(), "unanchored is centred");
        assert_eq!(settings.layer, Layer::Top, "the C++ default");
        assert_eq!(
            settings.keyboard_interactivity,
            KeyboardInteractivity::Exclusive
        );
        assert_eq!(settings.namespace.as_deref(), Some(layer::NAMESPACE));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn only_a_layer_message_converts_to_a_layer_request() {
        use iced_layershell::actions::LayerShellCustomActionWithId;
        let id = window::Id::unique();
        let request = layer::new_surface(id, &window::Settings::default());
        assert!(LayerShellCustomActionWithId::try_from(Message::Layer(request)).is_ok());
        assert!(matches!(
            LayerShellCustomActionWithId::try_from(Message::Opened(id)),
            Err(Message::Opened(_))
        ));
    }
}
