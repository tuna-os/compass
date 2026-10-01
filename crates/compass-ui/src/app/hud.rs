//! The HUD in the launcher: `NavigationController::showHud` closes the
//! window and, where the presentation can show one, puts up the pill
//! (`crate::hud`) for a moment.

use iced::widget::{container, row, text};
use iced::{Alignment, Border, Color, Element, Length, Theme};

use super::{Dismissal, LauncherApp, Message, Task};
use crate::hud::{Hud, Step, TEXT_MAX_WIDTH};

/// Puts `text` on the clipboard: the engine's, where there is an engine.
///
/// Not Iced's clipboard first, because its source lives on the window's
/// Wayland connection and both iced_layershell and iced_winit drop it with
/// the last surface: a copy that hides the launcher (most of them) was gone
/// before anything could paste it. The engine keeps its source for as long
/// as it runs. Where it cannot copy, [`Message::TextCopied`] falls back to
/// Iced's, which holds while the window does.
pub(super) fn copy_text(
    backend: Option<std::sync::Arc<dyn crate::backend::ApplicationBackend>>,
    text: String,
) -> Task<Message> {
    let Some(backend) = backend else {
        return iced::clipboard::write(text);
    };
    Task::perform(
        async move {
            let result = backend.copy_text(text.clone()).await;
            (text, result)
        },
        |(text, result)| Message::TextCopied { text, result },
    )
}

impl LauncherApp {
    /// The engine's answer to [`copy_text`]: nothing more to do, or copy
    /// through the window where it could not.
    pub(super) fn text_copied(text: String, result: Result<(), String>) -> Task<Message> {
        match result {
            Ok(()) => Task::none(),
            Err(reason) => {
                tracing::warn!(%reason, "the engine could not copy; copying through the window");
                iced::clipboard::write(text)
            }
        }
    }

    /// Hides the launcher and shows `hud`, as `showHud`: the window closes
    /// whether or not a HUD can be shown. A launcher that exits on dismissal
    /// has no process left to show one from.
    pub(super) fn show_hud(&mut self, hud: Hud) -> Task<Message> {
        let hidden = self.conceal();
        Task::batch([hidden, self.put_up_hud(hud)])
    }

    /// Clipboard write, then "Copied to clipboard", as `CopyToClipboardAction`.
    pub(super) fn copy_with_hud(&mut self, text: String) -> Task<Message> {
        Task::batch([
            copy_text(self.backend.clone(), text),
            self.show_hud(Hud::copied()),
        ])
    }

    /// The HUD without touching the launcher, for a message the engine sent
    /// (`WindowCommand::Hud`); `false` when this presentation has none.
    pub(super) fn put_up_hud_for_engine(&mut self, hud: Hud) -> (bool, Task<Message>) {
        if !self.hud.supported() || self.on_dismiss() == Dismissal::Exit {
            return (false, Task::none());
        }
        let task = self.put_up_hud(hud);
        (true, task)
    }

    fn put_up_hud(&mut self, hud: Hud) -> Task<Message> {
        if self.on_dismiss() == Dismissal::Exit {
            return Task::none();
        }
        match self.hud.show(hud, std::time::Instant::now()) {
            Step::Open(id) => crate::surface::open_hud(id),
            Step::Unsupported | Step::Update => Task::none(),
        }
    }

    /// A tick while the HUD's timer runs: closes it once the time is up.
    pub(super) fn hud_tick(&mut self, now: std::time::Instant) -> Task<Message> {
        self.hud
            .tick(now)
            .map_or_else(Task::none, iced::window::close)
    }

    /// Ticks while the HUD is up, so it closes on time.
    pub(super) fn hud_subscription(&self) -> Option<iced::Subscription<Message>> {
        self.hud
            .counting()
            .then(|| iced::time::every(std::time::Duration::from_millis(100)).map(Message::HudTick))
    }

    /// What the HUD shows, while it is up. For tests.
    #[must_use]
    pub fn hud_content(&self) -> Option<&Hud> {
        self.hud.content()
    }

    /// Lets the HUD show, as on a layer-shell presentation. For tests.
    #[must_use]
    pub fn with_hud(mut self, supported: bool) -> Self {
        self.hud.set_supported(supported);
        self
    }

    /// The view for surface `window`: the HUD's pill, the backdrop, or the
    /// launcher — nothing while a layer surface is still measuring its output
    /// (`window_sized`), which would show the card at the wrong size for a
    /// frame.
    pub fn view_for(&self, window: iced::window::Id) -> Element<'_, Message> {
        if self.hud.owns(window) {
            self.hud_view()
        } else if self.backdrop == Some(window) {
            iced::widget::mouse_area(container(text("")).width(Length::Fill).height(Length::Fill))
                .on_press(Message::BackdropPressed)
                .on_right_press(Message::BackdropPressed)
                .on_middle_press(Message::BackdropPressed)
                .into()
        } else if self.measuring == Some(window) {
            container(text("")).into()
        } else {
            self.view()
        }
    }

    /// `HudWindow.qml`: a pill in the background colour at 90%, the divider
    /// for its edge, a 16 px icon and one line of smaller text, elided at
    /// 270 px, centred in a transparent surface.
    fn hud_view(&self) -> Element<'_, Message> {
        let palette = self.palette();
        let Some(hud) = self.hud.content() else {
            return container(text("")).into();
        };
        let foreground = palette.text.to_iced();
        let size = 13.0;
        let mut line = row![].spacing(5).align_y(Alignment::Center);
        if let Some(icon) = hud.icon.as_deref() {
            if !hud.icon_is_builtin() {
                line = line.push(text(icon.to_owned()).size(size));
            } else if let Some(drawn) = self.builtin_svg(icon, foreground, 16.0) {
                line = line.push(drawn);
            }
        }
        // Elided rather than clipped, as `Layout.maximumWidth` with
        // `elide: Text.ElideRight`: a launch failure can be a long line.
        line = line.push(
            container(crate::elided::elided_shrink(
                hud.text.clone(),
                size,
                Some(self.font()),
                foreground,
            ))
            .max_width(TEXT_MAX_WIDTH),
        );
        let background = palette.surface.to_iced();
        let border = palette.border.to_iced();
        let pill = container(line)
            .padding([10, 15])
            .style(move |_: &Theme| container::Style {
                background: Some(
                    Color {
                        a: 0.9,
                        ..background
                    }
                    .into(),
                ),
                border: Border {
                    color: border,
                    width: 1.0,
                    radius: 999.0.into(),
                },
                ..container::Style::default()
            });
        container(pill)
            .width(Length::Fill)
            .height(Length::Fill)
            .center_x(Length::Fill)
            .center_y(Length::Fill)
            .into()
    }
}
