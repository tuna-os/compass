//! The first-run flow in the launcher: `OnboardingWindow.qml`'s three Linux
//! steps (welcome, "Make it your own", "Setup complete") with Back, Continue
//! and the step dots, Enter to continue and Escape to close.
//!
//! Finishing records the flow in `onboarding.json` and hides, as
//! `OnboardingWindow::finish`; closing does not record it, so the next start
//! shows it again, as closing the C++'s window does.

use iced::keyboard::{Key, key::Named};
use iced::widget::{Space, button, column, container, pick_list, row, text};
use iced::{Alignment, Border, Element, Length, Padding, Theme};

use super::{LauncherApp, Message, Page, Task};
use crate::onboarding_page::{OnboardingPage, ThemeOption};
use compass_core::onboarding::{self, Advance, Step};

/// Whether the platform binds the launcher's hotkey itself
/// (`Platform.supports("globalShortcuts")`). Compass binds only its fixed
/// toggle through the portal, with no recorder to change it, so the flow
/// takes the C++'s other branch: bind a key to `vicinae toggle`.
const SHORTCUTS_AVAILABLE: bool = false;

impl LauncherApp {
    /// Opens the flow at its first step, recording to `state_path`.
    pub(super) fn open_onboarding(&mut self, state_path: std::path::PathBuf) {
        let files = crate::theme::load_user_themes(&self.theme_dirs);
        self.page = Page::Onboarding(Box::new(OnboardingPage::new(state_path, files)));
    }

    /// Whether the flow is on screen. For tests.
    #[must_use]
    pub fn showing_onboarding(&self) -> bool {
        matches!(self.page, Page::Onboarding(_))
    }

    /// The step on screen, if the flow is. For tests.
    #[must_use]
    pub fn onboarding_step(&self) -> Option<Step> {
        match &self.page {
            Page::Onboarding(page) => Some(page.flow.step()),
            _ => None,
        }
    }

    /// The flow's keys: Enter continues, Escape closes it.
    pub(super) fn onboarding_key(&mut self, key: &Key) -> Task<Message> {
        match key.as_ref() {
            Key::Named(Named::Enter) => self.update(Message::OnboardingContinue),
            Key::Named(Named::Escape) => self.conceal(),
            _ => Task::none(),
        }
    }

    /// Handles the flow's messages.
    pub(super) fn onboarding_message(&mut self, message: Message) -> Task<Message> {
        let Page::Onboarding(page) = &mut self.page else {
            return Task::none();
        };
        match message {
            Message::OnboardingContinue => match page.flow.advance() {
                Advance::Next => Task::none(),
                Advance::Finish => {
                    let completed_at = jiff::Timestamp::now()
                        .round(jiff::Unit::Second)
                        .unwrap_or_else(|_| jiff::Timestamp::now())
                        .to_string();
                    if let Err(error) = onboarding::mark_completed(&page.state_path, &completed_at)
                    {
                        tracing::warn!(%error, path = %page.state_path.display(),
                            "could not write the onboarding state file");
                    }
                    self.conceal()
                }
            },
            Message::OnboardingBack => {
                page.flow.back();
                Task::none()
            }
            Message::OnboardingJump(position) => {
                page.flow.jump(position);
                Task::none()
            }
            Message::OnboardingTheme(ThemeOption(theme)) => {
                page.notice = None;
                let preview = self.update(Message::ThemePreview(theme));
                let Some(backend) = self.backend.clone() else {
                    return preview;
                };
                let name = theme.name().to_owned();
                Task::batch([
                    preview,
                    Task::perform(
                        async move { backend.set_theme(name).await },
                        Message::ThemeSaved,
                    ),
                ])
            }
            Message::OnboardingOpen(url) => {
                let Some(backend) = self.backend.clone() else {
                    return Task::none();
                };
                Task::perform(
                    async move { backend.open_url(url.to_owned()).await },
                    Message::OnboardingLinkOpened,
                )
            }
            Message::OnboardingLinkOpened(Err(reason)) => {
                page.notice = Some(reason);
                Task::none()
            }
            _ => Task::none(),
        }
    }

    /// The flow's card: the step, then Back, the dots and Continue.
    pub(super) fn onboarding_body<'a>(&'a self, page: &'a OnboardingPage) -> Element<'a, Message> {
        let palette = self.palette();
        let step = page.flow.step();
        let heading = text(step.heading())
            .font(iced::Font {
                weight: iced::font::Weight::Semibold,
                ..self.font()
            })
            .size(20)
            .color(palette.text.to_iced());
        let subtitle = text(step.subtitle(SHORTCUTS_AVAILABLE))
            .font(self.font())
            .size(14)
            .color(palette.muted.to_iced())
            .align_x(Alignment::Center);
        let small = |line: &'a str| {
            text(line)
                .font(self.font())
                .size(12)
                .color(palette.muted.to_iced())
        };
        let action = |label: &'a str, message: Message| {
            button(text(label).font(self.font()).size(13))
                .on_press(message)
                .padding(Padding::new(6.0).left(14).right(14))
        };

        let mut content = column![].spacing(8).align_x(Alignment::Center);
        if step == Step::Welcome
            && let Some(logo) = self.builtin_svg("vicinae", palette.text.to_iced(), 72.0)
        {
            content = content.push(container(logo).padding(Padding::new(0.0).bottom(12)));
        }
        content = content.push(heading).push(subtitle);
        match step {
            Step::Personalize => {
                let current = ThemeOption(self.theme_choice);
                let theme_row = row![
                    column![
                        text("Theme").font(self.font()).size(14),
                        small("Shared across the entire app."),
                    ]
                    .width(Length::Fill),
                    pick_list(
                        page.themes.as_slice(),
                        Some(current),
                        Message::OnboardingTheme
                    )
                    .text_size(13)
                    .width(Length::Fixed(200.0)),
                ]
                .align_y(Alignment::Center);
                let hotkey_row = row![
                    column![
                        text("Global hotkey").font(self.font()).size(14),
                        small("Bind a key to \"vicinae toggle\""),
                    ]
                    .width(Length::Fill),
                    action(
                        "Open Docs",
                        Message::OnboardingOpen(onboarding::HOTKEY_DOCS_URL)
                    ),
                ]
                .align_y(Alignment::Center);
                let border = palette.border.to_iced();
                content = content.push(
                    container(column![theme_row, hotkey_row].spacing(14))
                        .padding(14)
                        .width(Length::Fixed(480.0))
                        .style(move |_: &Theme| container::Style {
                            border: Border {
                                color: border,
                                width: 1.0,
                                radius: 8.0.into(),
                            },
                            ..container::Style::default()
                        }),
                );
            }
            Step::Complete => {
                content = content
                    .push(Space::new().height(16))
                    .push(small("Vicinae is open source software."))
                    .push(
                        row![
                            action("GitHub", Message::OnboardingOpen(onboarding::GITHUB_URL)),
                            action("Sponsor", Message::OnboardingOpen(onboarding::SPONSOR_URL)),
                        ]
                        .spacing(8),
                    );
            }
            Step::Welcome | Step::Permissions => {}
        }
        if let Some(notice) = &page.notice {
            content = content.push(self.notice(notice));
        }

        let accent = palette.accent.to_iced();
        let dim = iced::Color {
            a: 0.2,
            ..palette.text.to_iced()
        };
        let mut dots = row![].spacing(7);
        for position in 0..page.flow.count() {
            let color = if position == page.flow.position() {
                accent
            } else {
                dim
            };
            dots = dots.push(
                iced::widget::mouse_area(container(Space::new().width(7).height(7)).style(
                    move |_: &Theme| container::Style {
                        background: Some(color.into()),
                        border: Border {
                            radius: 3.5.into(),
                            ..Border::default()
                        },
                        ..container::Style::default()
                    },
                ))
                .on_press(Message::OnboardingJump(position)),
            );
        }
        let back: Element<'a, Message> = if page.flow.can_go_back() {
            action("Back", Message::OnboardingBack).into()
        } else {
            Space::new().into()
        };
        let footer = row![
            container(back).width(Length::Fill),
            dots,
            container(action(
                page.flow.primary_label(),
                Message::OnboardingContinue
            ))
            .width(Length::Fill)
            .align_x(Alignment::End),
        ]
        .align_y(Alignment::Center)
        .padding(16);

        column![
            container(content)
                .width(Length::Fill)
                .height(Length::Fixed(400.0))
                .center_x(Length::Fill)
                .center_y(Length::Fixed(400.0)),
            footer,
        ]
        .width(Length::Fill)
        .into()
    }
}
