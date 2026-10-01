//! Which kind of surface the launcher window is: an `xdg_toplevel` (GNOME,
//! and every compositor without `wlr-layer-shell`) or a layer surface (the
//! wlroots family, `PLAN.md` Phase 5 Track B).
//!
//! `LauncherApp` opens and closes one window and mostly does not care which.
//! The difference is in how it is asked for: under `iced::daemon` a window is
//! opened with `window::open`, while `iced_layershell`'s runtime ignores that
//! action and instead takes a `NewLayerShell` request, which reaches it as a
//! [`Message::Layer`] converted by the `TryFrom` impl below — so the app's own
//! `update` never sees one. Closing is the same in both: `iced_layershell`
//! maps `window::close` to removing the surface and reports it through
//! `window::close_events`, exactly as `iced_winit` does.
//!
//! Two things only a layer surface needs live here too: it opens at the
//! output's size to learn the room and is then fitted to it ([`fit`],
//! `fit_opened`), and with `close_on_focus_loss` on it gets a backdrop that
//! takes a click outside it (`open_backdrop`, [`backdrop_region`]), since a
//! surface holding the keyboard exclusively never loses the focus.
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

/// The gap kept between the launcher window and the edges of the space the
/// compositor gives it. The window's own shadow padding is inside this.
pub const SCREEN_MARGIN: f32 = 8.0;

/// The smallest the launcher window shrinks to, however little space there
/// is: below this the search field and a row or two no longer fit, and a
/// window cut off by the screen edge is no worse than one too small to use.
pub const MIN_WINDOW: iced::Size = iced::Size::new(360.0, 240.0);

/// The launcher window's size in `available` logical pixels: `wanted`, shrunk
/// to leave [`SCREEN_MARGIN`] on every side, and never below [`MIN_WINDOW`].
/// `None` (the space not known yet) is `wanted` as it is.
///
/// The fixed 768×608 window does not fit a 1366×768 panel at 150 % (911×512
/// logical) or a 1280×800 one at 200 % (640×400), and an oversized window is
/// centred, so the search field went off the top of the screen.
#[must_use]
pub fn fit(wanted: iced::Size, available: Option<iced::Size>) -> iced::Size {
    let Some(available) = available else {
        return wanted;
    };
    let room = |wanted: f32, available: f32, least: f32| {
        wanted
            .min(available - 2.0 * SCREEN_MARGIN)
            .max(least)
            .round()
    };
    iced::Size::new(
        room(wanted.width, available.width, MIN_WINDOW.width),
        room(wanted.height, available.height, MIN_WINDOW.height),
    )
}

/// Gives a layer surface opened by [`open`] its size, once its first
/// configure has said how much room the output has (see `layer::settings`).
/// Nothing for a toplevel, which is opened at its size.
pub(crate) fn fit_opened(id: window::Id, size: iced::Size) -> Task<Message> {
    match presentation() {
        #[cfg(target_os = "linux")]
        Presentation::LayerShell => {
            use iced_layershell::actions::{LayerShellCustomAction, LayerShellCustomActionWithId};
            use iced_layershell::reexport::Anchor;
            Task::done(Message::Layer(LayerShellCustomActionWithId::new(
                Some(id),
                LayerShellCustomAction::AnchorSizeChange(
                    Anchor::empty(),
                    (size.width.round() as u32, size.height.round() as u32),
                ),
            )))
        }
        _ => {
            let _ = (id, size);
            Task::none()
        }
    }
}

/// Whether a window this process opens starts at the full size of the space
/// it is given and is fitted afterwards (`fit_opened`): a layer surface,
/// whose compositor does not keep an oversized one on screen and does not
/// say how large the output is any other way.
#[must_use]
pub fn opens_to_measure() -> bool {
    presentation() == Presentation::LayerShell
}

/// Resizes the launcher window: `window::resize` for a toplevel, a size
/// change request for a layer surface, whose runtime ignores the former.
pub(crate) fn resize(id: window::Id, size: iced::Size) -> Task<Message> {
    match presentation() {
        #[cfg(target_os = "linux")]
        Presentation::LayerShell => {
            use iced_layershell::actions::{LayerShellCustomAction, LayerShellCustomActionWithId};
            Task::done(Message::Layer(LayerShellCustomActionWithId::new(
                Some(id),
                LayerShellCustomAction::SizeChange((
                    size.width.round() as u32,
                    size.height.round() as u32,
                )),
            )))
        }
        _ => window::resize(id, size),
    }
}

/// Opens the HUD's surface under `id`: a layer surface on the `top` layer,
/// centred, taking neither the keyboard nor the pointer, as
/// `HudWindowLayerShell.qml`. Nothing on a toplevel presentation, which has
/// no HUD (`crate::hud`).
pub(crate) fn open_hud(id: window::Id) -> Task<Message> {
    match presentation() {
        #[cfg(target_os = "linux")]
        Presentation::LayerShell => Task::done(Message::Layer(layer::new_hud_surface(id))),
        _ => {
            let _ = id;
            Task::none()
        }
    }
}

/// Opens the backdrop under `id`: a transparent layer surface over the whole
/// output, under the launcher and above windows, that takes a click outside
/// the launcher and no keyboard. Nothing on a toplevel presentation, where
/// losing the focus says the same thing.
pub(crate) fn open_backdrop(id: window::Id) -> Task<Message> {
    match presentation() {
        #[cfg(target_os = "linux")]
        Presentation::LayerShell => Task::done(Message::Layer(layer::new_backdrop_surface(id))),
        _ => {
            let _ = id;
            Task::none()
        }
    }
}

/// The backdrop's input region, as `(x, y, width, height)` rectangles: all of
/// `room` but the `launcher` centred in it, where the compositor puts an
/// unanchored layer surface (wlroots' `wlr_scene_layer_surface_v1_configure`
/// halves each in integers). Empty rectangles are left out.
#[must_use]
pub fn backdrop_region(room: iced::Size, launcher: iced::Size) -> Vec<(i32, i32, i32, i32)> {
    let (room_w, room_h) = (room.width.round() as i32, room.height.round() as i32);
    let (w, h) = (
        (launcher.width.round() as i32).min(room_w),
        (launcher.height.round() as i32).min(room_h),
    );
    let (x, y) = (room_w / 2 - w / 2, room_h / 2 - h / 2);
    [
        (0, 0, room_w, y),
        (0, y + h, room_w, room_h - y - h),
        (0, y, x, h),
        (x + w, y, room_w - x - w, h),
    ]
    .into_iter()
    .filter(|&(_, _, width, height)| width > 0 && height > 0)
    .collect()
}

/// Gives the backdrop `id` the input region [`backdrop_region`] describes,
/// so a click on the launcher reaches it whichever of the two the compositor
/// stacked on top. Nothing on a toplevel presentation, which has no backdrop.
pub(crate) fn cut_backdrop(
    id: window::Id,
    room: iced::Size,
    launcher: iced::Size,
) -> Task<Message> {
    match presentation() {
        #[cfg(target_os = "linux")]
        Presentation::LayerShell => {
            use iced_layershell::actions::{
                ActionCallback, LayerShellCustomAction, LayerShellCustomActionWithId,
            };
            let rectangles = backdrop_region(room, launcher);
            Task::done(Message::Layer(LayerShellCustomActionWithId::new(
                Some(id),
                LayerShellCustomAction::SetInputRegion(ActionCallback::new(move |region| {
                    for &(x, y, width, height) in &rectangles {
                        region.add(x, y, width, height);
                    }
                })),
            )))
        }
        _ => {
            let _ = (id, room, launcher);
            Task::none()
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
    /// (`layer_effects "compass" …` in Hyprland, `layer-rule` in niri). The
    /// C++ engine's is `vicinae`.
    pub const NAMESPACE: &str = "compass";

    /// The launcher as a layer surface, taking the keyboard exclusively,
    /// which is what a launcher summoned by a hotkey needs — `OnDemand` would
    /// leave the keys with whatever had them until the user clicks. The `top`
    /// layer and exclusive keyboard are the C++ launcher's defaults
    /// (`LayerShellConfig`); its config keys that change them are not ported
    /// yet (PARITY.md, wlroots).
    ///
    /// It opens anchored to every edge with no size, so the compositor's
    /// first configure is the room the output has, less its panels. The app
    /// then asks for the window's own size, fitted to that, anchored to
    /// nothing so the compositor centres it (`super::fit_opened`). Asking for
    /// the size up front instead left the window larger than a small or
    /// scaled output, and a compositor centres it regardless (TIL-04). The
    /// requested size in `_settings` is the app's to fit, so it is not used.
    pub fn settings(_settings: &window::Settings) -> NewLayerShellSettings {
        NewLayerShellSettings {
            size: None,
            layer: Layer::Top,
            anchor: Anchor::all(),
            exclusive_zone: None,
            margin: None,
            keyboard_interactivity: KeyboardInteractivity::Exclusive,
            output_option: OutputOption::Active,
            events_transparent: false,
            namespace: Some(NAMESPACE.to_owned()),
        }
    }

    /// The HUD as a layer surface: centred on the active output, above
    /// windows, with no keyboard interactivity so the application the
    /// launcher hid back to keeps its focus, and transparent to the pointer.
    pub fn hud_settings() -> NewLayerShellSettings {
        NewLayerShellSettings {
            size: Some(crate::hud::SURFACE_SIZE),
            layer: Layer::Top,
            anchor: Anchor::empty(),
            exclusive_zone: None,
            margin: None,
            keyboard_interactivity: KeyboardInteractivity::None,
            output_option: OutputOption::Active,
            events_transparent: true,
            namespace: Some(crate::hud::NAMESPACE.to_owned()),
        }
    }

    /// The backdrop's namespace.
    pub const BACKDROP_NAMESPACE: &str = "compass-backdrop";

    /// The backdrop as a layer surface: the room the launcher is centred in
    /// (the output less its panels, as the launcher measures it), on the
    /// launcher's layer, taking the pointer and not the keyboard.
    pub fn backdrop_settings() -> NewLayerShellSettings {
        NewLayerShellSettings {
            size: None,
            layer: Layer::Top,
            anchor: Anchor::all(),
            exclusive_zone: None,
            margin: None,
            keyboard_interactivity: KeyboardInteractivity::None,
            output_option: OutputOption::Active,
            events_transparent: false,
            namespace: Some(BACKDROP_NAMESPACE.to_owned()),
        }
    }

    pub fn new_backdrop_surface(id: window::Id) -> LayerShellCustomActionWithId {
        LayerShellCustomActionWithId::new(
            None,
            LayerShellCustomAction::NewLayerShell {
                settings: backdrop_settings(),
                id,
            },
        )
    }

    pub fn new_hud_surface(id: window::Id) -> LayerShellCustomActionWithId {
        LayerShellCustomActionWithId::new(
            None,
            LayerShellCustomAction::NewLayerShell {
                settings: hud_settings(),
                id,
            },
        )
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
    fn the_layer_surface_opens_to_measure_the_output_on_top_and_takes_the_keyboard() {
        use iced_layershell::reexport::{Anchor, KeyboardInteractivity, Layer};
        let settings = layer::settings(&window::Settings {
            size: iced::Size::new(768.0, 520.0),
            ..window::Settings::default()
        });
        assert_eq!(settings.size, None, "the compositor says how much room");
        assert_eq!(settings.anchor, Anchor::all());
        assert_eq!(settings.layer, Layer::Top, "the C++ default");
        assert_eq!(
            settings.keyboard_interactivity,
            KeyboardInteractivity::Exclusive
        );
        assert_eq!(settings.namespace.as_deref(), Some(layer::NAMESPACE));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn the_hud_surface_takes_no_keyboard_and_no_pointer() {
        use iced_layershell::reexport::{Anchor, KeyboardInteractivity, Layer};
        let settings = layer::hud_settings();
        assert_eq!(settings.size, Some(crate::hud::SURFACE_SIZE));
        assert_eq!(settings.anchor, Anchor::empty(), "centred, as AnchorNone");
        assert_eq!(settings.layer, Layer::Top);
        assert_eq!(settings.keyboard_interactivity, KeyboardInteractivity::None);
        assert!(settings.events_transparent);
        assert_eq!(settings.namespace.as_deref(), Some("compass-hud"));
    }

    #[test]
    fn the_window_fits_small_and_scaled_outputs_and_keeps_its_size_on_large_ones() {
        let wanted = iced::Size::new(768.0, 608.0);
        // 1920×1080 at 100 %: room to spare.
        assert_eq!(fit(wanted, Some(iced::Size::new(1920.0, 1080.0))), wanted);
        // 1366×768 at 150 %, the QA's laptop: 911×512 logical.
        assert_eq!(
            fit(wanted, Some(iced::Size::new(911.0, 512.0))),
            iced::Size::new(768.0, 496.0)
        );
        // 1280×800 at 200 %: 640×400.
        assert_eq!(
            fit(wanted, Some(iced::Size::new(640.0, 400.0))),
            iced::Size::new(624.0, 384.0)
        );
        // Not known yet: as asked.
        assert_eq!(fit(wanted, None), wanted);
        // Absurdly small: no smaller than usable.
        assert_eq!(fit(wanted, Some(iced::Size::new(200.0, 100.0))), MIN_WINDOW);
    }

    #[test]
    fn the_backdrop_takes_clicks_everywhere_but_on_the_launcher() {
        let room = iced::Size::new(1280.0, 800.0);
        let launcher = iced::Size::new(768.0, 608.0);
        // Centred as wlroots centres it: 256..1024 across, 96..704 down.
        assert_eq!(
            backdrop_region(room, launcher),
            [
                (0, 0, 1280, 96),
                (0, 704, 1280, 96),
                (0, 96, 256, 608),
                (1024, 96, 256, 608)
            ]
        );
        // Odd sizes halve in integers, each on its own.
        let odd = backdrop_region(iced::Size::new(911.0, 512.0), iced::Size::new(767.0, 495.0));
        assert_eq!(odd[0], (0, 0, 911, 9));
        assert_eq!(odd[2], (0, 9, 72, 495));
        // A launcher as large as the room leaves nothing to click.
        assert!(backdrop_region(room, room).is_empty());
    }

    #[test]
    fn a_toplevel_is_opened_at_its_size() {
        assert!(!opens_to_measure());
        assert_eq!(
            fit_opened(window::Id::unique(), iced::Size::new(1.0, 1.0)).units(),
            0
        );
    }

    #[test]
    fn a_toplevel_presentation_opens_no_hud_surface() {
        assert_eq!(presentation(), Presentation::Toplevel);
        assert_eq!(open_hud(window::Id::unique()).units(), 0);
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
