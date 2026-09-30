//! The HUD in the launcher: `NavigationController::showHud` closes the
//! window and, where the presentation can show one, puts up the pill
//! (`crate::hud`) for a moment.

use iced::widget::{container, row, text};
use iced::{Alignment, Border, Color, Element, Length, Theme};

use super::{Dismissal, LauncherApp, Message, Task};
use crate::hud::{Hud, Step, TEXT_MAX_WIDTH};

impl LauncherApp {
    /// Hides the launcher and shows `hud`, as `showHud`: the window closes
    /// whether or not a HUD can be shown. A launcher that exits on dismissal
    /// has no process left to show one from.
    pub(super) fn show_hud(&mut self, hud: Hud) -> Task<Message> {
        let hidden = self.conceal();
        Task::batch([hidden, self.put_up_hud(hud)])
    }

    /// Clipboard write, then "Copied to clipboard", as `CopyToClipboardAction`.
    pub(super) fn copy_with_hud(&mut self, text: String) -> Task<Message> {
        Task::batch([iced::clipboard::write(text), self.show_hud(Hud::copied())])
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

    /// The view for surface `window`: the HUD's pill or the launcher.
    pub fn view_for(&self, window: iced::window::Id) -> Element<'_, Message> {
        if self.hud.owns(window) {
            self.hud_view()
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
        line = line.push(
            container(
                text(hud.text.clone())
                    .font(self.font())
                    .size(size)
                    .color(foreground)
                    .wrapping(iced::widget::text::Wrapping::None),
            )
            .max_width(TEXT_MAX_WIDTH)
            .clip(true),
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
