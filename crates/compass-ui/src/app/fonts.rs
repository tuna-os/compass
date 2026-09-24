//! Browse Fonts in the launcher: the list, its category filter, and the
//! specimen.
//!
//! A child module of `app` so it can reach the launcher's state without
//! widening it; the rest is in [`crate::fonts_page`].

use iced::keyboard::{Key, Modifiers, key::Named};

use super::{
    Direction, Element, LauncherApp, Length, Message, Padding, Page, PanelSection, PanelState,
    Task, chord_direction, column, container, focus_search, mouse_area, next_selection, row,
    scrollable, text,
};
use crate::action_panel::Action;
use crate::fonts_page::{self, FontPreviewPage, FontsPage, SpecimenLine, Status};

const PREVIEW: &str = "font.preview";
const COPY_FAMILY: &str = "font.copy-family";

impl LauncherApp {
    /// Opens Browse Fonts and asks for the families.
    pub(super) fn open_browse_fonts(&mut self) -> Task<Message> {
        let page = self.parked_fonts.take().unwrap_or_default();
        let loaded = page.status == Status::Ready;
        self.page = Page::Fonts(page);
        if loaded {
            return focus_search();
        }
        let Some(backend) = self.backend.clone() else {
            if let Page::Fonts(page) = &mut self.page {
                page.apply(Err("Browse Fonts needs the Compass engine".to_owned()));
            }
            return focus_search();
        };
        Task::batch([
            Task::perform(
                async move { backend.list_fonts().await },
                Message::FontsLoaded,
            ),
            focus_search(),
        ])
    }

    /// Opens the selected family's specimen.
    fn preview_selected_font(&mut self) -> Task<Message> {
        let Page::Fonts(page) = &self.page else {
            return Task::none();
        };
        let (Some(family), Some(backend)) = (page.selected_family(), self.backend.clone()) else {
            return Task::none();
        };
        let (name, family) = (family.name.clone(), family.family.clone());
        self.panel = None;
        Task::perform(
            {
                let name = name.clone();
                async move { backend.font_specimen(name).await }
            },
            move |result| Message::FontSpecimenLoaded {
                name: name.clone(),
                family: family.clone(),
                result,
            },
        )
    }

    /// The panel over the selected family: `FontBrowserViewHost`'s actions,
    /// less "Set as vicinae font".
    pub(super) fn open_font_panel(&mut self) -> Option<Task<Message>> {
        let Page::Fonts(page) = &self.page else {
            return None;
        };
        page.selected_family()?;
        self.panel = Some(PanelState::new(vec![PanelSection {
            name: String::new(),
            actions: vec![
                Action::new(compass_core::font_browser::PREVIEW_TITLE)
                    .with_id(PREVIEW)
                    .with_shortcut("enter"),
                Action::new(compass_core::font_browser::COPY_FAMILY_TITLE).with_id(COPY_FAMILY),
            ],
        }]));
        Some(iced::widget::operation::focus(super::PANEL_INPUT))
    }

    /// Runs a font panel action, if `id` is one.
    pub(super) fn font_panel_action(&mut self, id: &str) -> Option<Task<Message>> {
        let Page::Fonts(page) = &self.page else {
            return None;
        };
        let task = match id {
            PREVIEW => self.preview_selected_font(),
            COPY_FAMILY => {
                let family = page.selected_family()?.family.clone();
                self.panel = None;
                return Some(Task::batch([
                    iced::clipboard::write(family),
                    self.conceal(),
                ]));
            }
            _ => return None,
        };
        self.panel = None;
        Some(task)
    }

    /// The list's keys.
    pub(super) fn fonts_page_key(&mut self, key: &Key, modifiers: Modifiers) -> Task<Message> {
        let Page::Fonts(page) = &mut self.page else {
            return Task::none();
        };
        let direction = match key.as_ref() {
            Key::Named(Named::ArrowDown) => Some(Direction::Down),
            Key::Named(Named::ArrowUp) => Some(Direction::Up),
            Key::Named(Named::Escape) => return self.update(Message::Back),
            Key::Named(Named::Enter) => return self.preview_selected_font(),
            _ => chord_direction(self.keybinding, key.as_ref(), modifiers),
        };
        if let Some(direction) = direction {
            page.selected = next_selection(
                page.shown().1.len(),
                page.selected,
                direction,
                self.wrap_navigation,
            );
            return crate::scroll::reveal_root_selection();
        }
        Task::none()
    }

    /// The specimen's keys: Escape goes back to the list, as it was.
    pub(super) fn font_preview_key(&mut self, key: &Key) -> Task<Message> {
        if key.as_ref() == Key::Named(Named::Escape) {
            return self.open_browse_fonts();
        }
        Task::none()
    }

    /// Handles the Browse Fonts messages.
    pub(super) fn font_message(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::FontsLoaded(result) => {
                if let Page::Fonts(page) = &mut self.page {
                    page.apply(result);
                }
                crate::scroll::reveal_root_selection()
            }
            Message::FontsQueryChanged(query) => {
                if let Page::Fonts(page) = &mut self.page {
                    page.query = query;
                    page.notice = None;
                    page.refilter();
                }
                crate::scroll::reveal_root_selection()
            }
            Message::FontsCategoryChanged(option) => {
                if let Page::Fonts(page) = &mut self.page {
                    page.set_category(&option);
                }
                Task::batch([focus_search(), crate::scroll::reveal_root_selection()])
            }
            Message::FontSelected(position) => {
                if let Page::Fonts(page) = &mut self.page
                    && position < page.shown().1.len()
                {
                    page.selected = position;
                    return self.preview_selected_font();
                }
                Task::none()
            }
            Message::FontSpecimenLoaded {
                name,
                family,
                result: Ok(markdown),
            } => {
                if let Page::Fonts(page) = &self.page {
                    self.parked_fonts = Some(page.clone());
                }
                self.page = Page::FontPreview(FontPreviewPage {
                    name,
                    family,
                    lines: fonts_page::specimen_lines(&markdown),
                });
                Task::none()
            }
            Message::FontSpecimenLoaded {
                result: Err(reason),
                ..
            } => {
                if let Page::Fonts(page) = &mut self.page {
                    page.notice = Some(reason);
                }
                Task::none()
            }
            _ => Task::none(),
        }
    }

    /// The list's body: the category filter, the heading, and each family
    /// named in its own font beside its glyph.
    pub(super) fn fonts_body<'a>(&'a self, page: &'a FontsPage) -> Element<'a, Message> {
        match &page.status {
            Status::Loading => return self.notice("Reading the installed fonts…"),
            Status::Failed(reason) => return self.notice(reason),
            Status::Ready => {}
        }
        let palette = self.palette();
        let (title, families) = page.shown();
        let selected_option = page
            .category
            .clone()
            .unwrap_or_else(|| compass_core::font_browser::ALL_OPTION.to_owned());
        let filter = iced::widget::pick_list(
            page.options.clone(),
            Some(selected_option),
            Message::FontsCategoryChanged,
        )
        .text_size(12);
        let heading = text(title.to_owned())
            .font(self.font())
            .size(12)
            .color(palette.muted.to_iced());
        let mut list = column![
            container(row![
                heading,
                iced::widget::Space::new().width(Length::Fill),
                filter
            ])
            .padding(Padding::new(4.0).left(10).right(10))
        ]
        .spacing(f32::from(self.geometry.row_spacing));
        if families.is_empty() {
            return column![list, self.notice("No fonts match")].into();
        }
        for (position, family) in families.iter().enumerate() {
            let selected = position == page.selected;
            let font = iced::Font::with_name(fonts_page::static_family(&family.family));
            let preview = compass_core::font_browser::preview(family);
            let glyph = text(preview.glyph)
                .font(if preview.family.is_empty() {
                    self.font()
                } else {
                    font
                })
                .size(18)
                .width(Length::Fixed(f32::from(self.geometry.icon_size)));
            let item = self.list_row(
                glyph.into(),
                family.name.clone(),
                self.subtitles
                    .then(|| page.primaries.get(&family.name).cloned())
                    .flatten(),
                selected,
            );
            let item: Element<Message> = mouse_area(item)
                .on_press(Message::FontSelected(position))
                .into();
            let item: Element<Message> = if selected {
                container(item).id(crate::scroll::ROOT_SELECTION).into()
            } else {
                item
            };
            list = list.push(item);
        }
        let rows = scrollable(container(list).padding(Padding::new(6.0).top(8)))
            .id(crate::scroll::ROOT_RESULTS)
            .height(Length::Shrink);
        match &page.notice {
            Some(notice) => column![rows, self.notice(notice)].into(),
            None => rows.into(),
        }
    }

    /// The specimen's body, drawn in the font.
    pub(super) fn font_preview_body<'a>(
        &'a self,
        page: &'a FontPreviewPage,
    ) -> Element<'a, Message> {
        let family = fonts_page::static_family(&page.family);
        let font = iced::Font::with_name(family);
        let palette = self.palette();
        let mut body = column![
            text(compass_core::font_browser::navigation_title(
                "Browse Fonts",
                Some(&page.name)
            ))
            .font(self.font())
            .size(12)
            .color(palette.muted.to_iced())
        ]
        .spacing(8);
        for line in &page.lines {
            let element: Element<Message> = match line {
                SpecimenLine::Heading(sample) => text(sample.clone()).font(font).size(28).into(),
                SpecimenLine::Regular(sample) => text(sample.clone()).font(font).size(16).into(),
                SpecimenLine::Bold(sample) => text(sample.clone())
                    .font(iced::Font {
                        weight: iced::font::Weight::Bold,
                        ..font
                    })
                    .size(16)
                    .into(),
                SpecimenLine::Italic(sample) => text(sample.clone())
                    .font(iced::Font {
                        style: iced::font::Style::Italic,
                        ..font
                    })
                    .size(16)
                    .into(),
                SpecimenLine::Rule => iced::widget::rule::horizontal(1).into(),
            };
            body = body.push(element);
        }
        scrollable(container(body).padding(Padding::new(14.0)))
            .id(crate::scroll::ROOT_RESULTS)
            .height(Length::Shrink)
            .into()
    }
}
