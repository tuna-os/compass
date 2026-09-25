//! Browse Fonts in the launcher: the list, its category filter, and the
//! specimen.
//!
//! A child module of `app` so it can reach the launcher's state without
//! widening it; the rest is in [`crate::fonts_page`].

use iced::keyboard::{Key, Modifiers, key::Named};

use super::{
    Direction, Element, LauncherApp, Length, Message, Padding, Page, PanelSection, PanelState,
    Task, chord_direction, column, container, focus_search, mouse_area, row, scrollable, text,
};
use crate::action_panel::Action;
use crate::fonts_page::{self, FontPreviewPage, FontsPage, GridMove, SpecimenLine, Status};
use compass_core::font_browser::COLUMNS;

const PREVIEW: &str = "font.preview";
const COPY_FAMILY: &str = "font.copy-family";
const SET_APP_FONT: &str = "font.set-app-font";

/// A tile's height; the C++ grid's cells are square (`ASPECT_RATIO`), and a
/// sixth of the card's width is about this.
const TILE_HEIGHT: f32 = 104.0;

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

    /// The panel over the selected family: `FontBrowserViewHost`'s actions.
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
                Action::new(compass_core::font_browser::SET_APP_FONT_TITLE).with_id(SET_APP_FONT),
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
            SET_APP_FONT => {
                let family = page.selected_family()?.family.clone();
                self.panel = None;
                let Some(backend) = self.backend.clone() else {
                    if let Page::Fonts(page) = &mut self.page {
                        page.notice = Some("Setting the font needs the Compass engine".to_owned());
                    }
                    return Some(Task::none());
                };
                return Some(Task::batch([
                    Task::perform(
                        {
                            let family = family.clone();
                            async move { backend.set_font(family).await }
                        },
                        move |result| Message::FontSet(result.map(|()| family.clone())),
                    ),
                    focus_search(),
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
        let step = match key.as_ref() {
            Key::Named(Named::ArrowDown) => Some(GridMove::Down),
            Key::Named(Named::ArrowUp) => Some(GridMove::Up),
            Key::Named(Named::ArrowLeft) => Some(GridMove::Left),
            Key::Named(Named::ArrowRight) => Some(GridMove::Right),
            Key::Named(Named::Escape) => return self.update(Message::Back),
            Key::Named(Named::Enter) => return self.preview_selected_font(),
            _ => chord_direction(self.keybinding, key.as_ref(), modifiers).map(|direction| {
                match direction {
                    Direction::Up => GridMove::Up,
                    Direction::Down => GridMove::Down,
                }
            }),
        };
        if let Some(step) = step {
            page.selected = fonts_page::grid_step(
                page.shown().1.len(),
                page.selected,
                COLUMNS,
                step,
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
                let saved = self
                    .view_memory
                    .get(crate::view_memory::FONT_CATEGORY)
                    .map(str::to_owned);
                if let Page::Fonts(page) = &mut self.page {
                    page.apply(result);
                    // `restoreCategoryFilter`: a remembered category some
                    // font still has.
                    if let Some(index) =
                        compass_core::font_browser::index_for_saved(&page.options, saved.as_deref())
                    {
                        let option = page.options[index].clone();
                        page.set_category(&option);
                    }
                }
                crate::scroll::reveal_root_selection()
            }
            Message::FontSet(result) => {
                match result {
                    Ok(family) => {
                        // The configured family replaces the desktop's.
                        self.typography_link = None;
                        self.font_family = Some(family);
                    }
                    Err(reason) => {
                        if let Page::Fonts(page) = &mut self.page {
                            page.notice = Some(reason);
                        }
                    }
                }
                Task::none()
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
                self.view_memory
                    .set(crate::view_memory::FONT_CATEGORY, &option);
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
        for (row_index, chunk) in families.chunks(COLUMNS).enumerate() {
            let mut line = row![].spacing(6);
            for (column, family) in chunk.iter().enumerate() {
                let position = row_index * COLUMNS + column;
                line = line.push(self.font_tile(family, position == page.selected, position));
            }
            for _ in chunk.len()..COLUMNS {
                line = line.push(iced::widget::Space::new().width(Length::Fill));
            }
            list = list.push(line);
        }
        let rows = scrollable(container(list).padding(Padding::new(6.0).top(8)))
            .id(crate::scroll::ROOT_RESULTS)
            .height(Length::Shrink);
        match &page.notice {
            Some(notice) => column![rows, self.notice(notice)].into(),
            None => rows.into(),
        }
    }

    /// One tile of the grid: the family's glyph drawn in it, its name under.
    fn font_tile<'a>(
        &'a self,
        family: &'a compass_core::font_browser::FontFamily,
        selected: bool,
        position: usize,
    ) -> Element<'a, Message> {
        let palette = self.palette();
        let preview = compass_core::font_browser::preview(family);
        let glyph_font = if preview.family.is_empty() {
            self.font()
        } else {
            iced::Font::with_name(fonts_page::static_family(&family.family))
        };
        let colour = if selected {
            palette.selection_text
        } else {
            palette.text
        };
        let tile = column![
            container(text(preview.glyph).font(glyph_font).size(34).color(
                if preview.fill_foreground {
                    colour.to_iced()
                } else {
                    palette.text.to_iced()
                }
            ))
            .width(Length::Fill)
            .height(Length::Fill)
            .align_x(iced::Alignment::Center)
            .align_y(iced::Alignment::Center),
            text(family.name.clone())
                .font(self.font())
                .size(11)
                .color(if selected {
                    palette.selection_text.to_iced()
                } else {
                    palette.muted.to_iced()
                })
                .wrapping(iced::widget::text::Wrapping::None)
                .width(Length::Fill)
                .align_x(iced::Alignment::Center),
        ]
        .spacing(4);
        let tile = container(tile)
            .width(Length::Fill)
            .height(Length::Fixed(TILE_HEIGHT))
            .padding(6)
            .clip(true)
            .style(move |_: &iced::Theme| container::Style {
                background: Some(
                    if selected {
                        palette.selection
                    } else {
                        palette.field
                    }
                    .to_iced()
                    .into(),
                ),
                border: iced::Border {
                    radius: 8.0.into(),
                    ..iced::Border::default()
                },
                ..container::Style::default()
            });
        let tile: Element<'a, Message> = mouse_area(tile)
            .on_press(Message::FontSelected(position))
            .into();
        if selected {
            container(tile)
                .id(crate::scroll::ROOT_SELECTION)
                .width(Length::Fill)
                .into()
        } else {
            tile
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
