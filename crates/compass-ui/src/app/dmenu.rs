//! The dmenu view in the launcher: shown when the engine pushes a
//! `vicinae dmenu` list, answered when the person chooses or dismisses it.
//!
//! A child module of `app` so it can reach the launcher's state without
//! widening it; the decisions that do not need that state are in
//! [`crate::dmenu_page`].

use iced::keyboard::{Key, Modifiers, key::Named};

use super::{
    Direction, Element, LauncherApp, Length, Message, Padding, Page, PanelSection, PanelState,
    Task, chord_direction, column, container, mouse_area, next_selection, row, scrollable, text,
};
use crate::action_panel::Action;
use crate::design::{GEOMETRY, SHADOW_PADDING};
use crate::dmenu_page::{self, DmenuPage, Status};

const SELECT: &str = "dmenu.select";
const PASS_TEXT: &str = "dmenu.pass-text";
const SELECT_AND_COPY: &str = "dmenu.select-copy";

impl LauncherApp {
    /// The card size the dmenu list showing asks for with `--width` and
    /// `--height`, the side not given keeping the launcher's own.
    pub(super) fn dmenu_card_size(&self) -> Option<(u32, u32)> {
        let Page::Dmenu(page) = &self.page else {
            return None;
        };
        page.window_size((
            u32::from(GEOMETRY.card_width),
            u32::from(GEOMETRY.card_max_height),
        ))
    }

    /// Resizes the open window to what the dmenu list asks for, or back to
    /// the launcher's own size after a list that asked (`requestWindowSize`).
    pub(super) fn apply_dmenu_size(&mut self) -> Task<Message> {
        let Some(id) = self.window else {
            return Task::none();
        };
        let wanted = self.dmenu_card_size();
        if wanted == self.resized_to {
            return Task::none();
        }
        self.resized_to = wanted;
        let (width, height) = wanted.unwrap_or((
            u32::from(GEOMETRY.card_width),
            u32::from(GEOMETRY.card_max_height),
        ));
        let pad = 2 * u32::from(SHADOW_PADDING);
        crate::surface::resize(
            id,
            iced::Size::new((width + pad) as f32, (height + pad) as f32),
        )
    }

    /// Puts up the dmenu view for `token` and asks the engine for its list.
    pub(super) fn start_dmenu(&mut self, token: u64) -> Task<Message> {
        self.panel = None;
        let _ = self.close_extension_view();
        self.page = Page::Dmenu(DmenuPage::loading(token));
        let Some(backend) = self.backend.clone() else {
            if let Page::Dmenu(page) = &mut self.page {
                page.apply(Err("dmenu needs the Compass engine".to_owned()));
            }
            return Task::none();
        };
        Task::perform(
            async move { backend.fetch_dmenu(token).await },
            move |result| Message::DmenuLoaded { token, result },
        )
    }

    /// Sends the choice (or the dismissal, as `None`) for the dmenu view,
    /// once.
    fn answer_dmenu(&mut self, output: Option<String>) -> Task<Message> {
        let Page::Dmenu(page) = &mut self.page else {
            return Task::none();
        };
        if page.answered {
            return Task::none();
        }
        page.answered = true;
        let token = page.token;
        let Some(backend) = self.backend.clone() else {
            return Task::none();
        };
        Task::perform(
            async move { backend.choose_dmenu(token, output).await },
            Message::DmenuChosen,
        )
    }

    /// Dismisses a dmenu list that was not answered, when the view goes away
    /// for any reason: Escape, the window hiding, another command.
    pub(super) fn cancel_dmenu(&mut self) -> Task<Message> {
        self.answer_dmenu(None)
    }

    /// Chooses: the selected entry (or its index), else the search text; and
    /// hides the launcher, as `selectEntry` closes the window.
    fn choose_dmenu(&mut self, output: String, copy: Option<String>) -> Task<Message> {
        let answer = self.answer_dmenu(Some(output));
        let copied = copy.map_or_else(Task::none, iced::clipboard::write);
        Task::batch([answer, copied, self.conceal()])
    }

    /// The view's keys: navigation, Enter to choose, Escape to dismiss.
    pub(super) fn dmenu_page_key(&mut self, key: &Key, modifiers: Modifiers) -> Task<Message> {
        let Page::Dmenu(page) = &mut self.page else {
            return Task::none();
        };
        let direction = match key.as_ref() {
            Key::Named(Named::ArrowDown) => Some(Direction::Down),
            Key::Named(Named::ArrowUp) => Some(Direction::Up),
            Key::Named(Named::Escape) => return self.update(Message::Back),
            Key::Named(Named::Enter) => {
                let output = page.selected_output();
                return self.choose_dmenu(output, None);
            }
            _ => chord_direction(self.keybinding, key.as_ref(), modifiers),
        };
        if let Some(direction) = direction {
            page.selected = next_selection(
                page.shown.len(),
                page.selected,
                direction,
                self.wrap_navigation,
            );
            page.refresh_preview();
            return crate::scroll::reveal_root_selection();
        }
        Task::none()
    }

    /// The panel over the dmenu view: `DMenuSection::actionPanel`, or the
    /// empty list's "Pass search text".
    pub(super) fn open_dmenu_panel(&mut self) -> Option<Task<Message>> {
        let Page::Dmenu(page) = &self.page else {
            return None;
        };
        let actions = if page.selected_entry().is_some() {
            vec![
                Action::new(if page.list.output_index {
                    "Select entry (index)"
                } else {
                    "Select entry"
                })
                .with_id(SELECT)
                .with_shortcut("enter"),
                Action::new("Pass search text").with_id(PASS_TEXT),
                Action::new("Select and copy entry").with_id(SELECT_AND_COPY),
            ]
        } else {
            vec![
                Action::new("Pass search text")
                    .with_id(PASS_TEXT)
                    .with_shortcut("enter"),
            ]
        };
        self.panel = Some(PanelState::new(vec![PanelSection {
            name: String::new(),
            actions,
        }]));
        Some(iced::widget::operation::focus(super::PANEL_INPUT))
    }

    /// Runs a dmenu panel action, if `id` is one.
    pub(super) fn dmenu_panel_action(&mut self, id: &str) -> Option<Task<Message>> {
        let Page::Dmenu(page) = &self.page else {
            return None;
        };
        let (output, copy) = match id {
            SELECT => (page.selected_output(), None),
            PASS_TEXT => (page.query.clone(), None),
            SELECT_AND_COPY => {
                let entry = page.selected_entry()?.to_owned();
                (entry.clone(), Some(entry))
            }
            _ => return None,
        };
        self.panel = None;
        Some(self.choose_dmenu(output, copy))
    }

    /// Handles the dmenu messages.
    pub(super) fn dmenu_message(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::DmenuLoaded { token, result } => {
                if let Page::Dmenu(page) = &mut self.page
                    && page.token == token
                {
                    page.apply(result);
                }
                Task::batch([
                    self.apply_dmenu_size(),
                    crate::scroll::reveal_root_selection(),
                ])
            }
            Message::DmenuQueryChanged(query) => {
                if let Page::Dmenu(page) = &mut self.page {
                    page.query = query;
                    page.refilter();
                }
                crate::scroll::reveal_root_selection()
            }
            Message::DmenuSelected(position) => {
                let Page::Dmenu(page) = &mut self.page else {
                    return Task::none();
                };
                if position >= page.shown.len() {
                    return Task::none();
                }
                page.selected = position;
                let output = page.selected_output();
                self.choose_dmenu(output, None)
            }
            Message::DmenuChosen(Err(reason)) => {
                tracing::warn!(%reason, "could not answer a dmenu list");
                Task::none()
            }
            _ => Task::none(),
        }
    }

    /// The view's body: the list, quick look beside it when the selected
    /// entry is a file, and the footer unless `--no-footer`.
    pub(super) fn dmenu_body<'a>(&'a self, page: &'a DmenuPage) -> Element<'a, Message> {
        let list = self.dmenu_list(page);
        let body: Element<'a, Message> = match &page.preview {
            Some(preview) => row![
                container(list).width(Length::FillPortion(super::preview::LIST_PORTION)),
                self.file_preview_pane(preview, !page.list.no_metadata),
            ]
            .into(),
            None => list,
        };
        if page.list.no_footer || page.status != Status::Ready {
            return body;
        }
        let primary = match (page.selected_entry(), page.list.output_index) {
            (None, _) => "Pass search text",
            (Some(_), false) => "Select entry",
            (Some(_), true) => "Select entry (index)",
        };
        column![
            body,
            self.footer(page.list.navigation_title.as_deref(), primary)
        ]
        .into()
    }

    /// The list itself.
    fn dmenu_list<'a>(&'a self, page: &'a DmenuPage) -> Element<'a, Message> {
        match &page.status {
            Status::Loading => return self.notice("Loading entries…"),
            Status::Failed(reason) => return self.notice(reason),
            Status::Ready => {}
        }
        let quick_look = !page.list.no_quick_look;
        let mut list = column![].spacing(f32::from(self.geometry.row_spacing));
        if let Some(heading) = page.heading() {
            list = list.push(
                container(
                    text(heading)
                        .font(self.font())
                        .size(12)
                        .color(self.palette().muted.to_iced()),
                )
                .padding(Padding::new(4.0).left(10)),
            );
        }
        if page.shown.is_empty() {
            return column![
                list,
                self.notice("No entries match. Enter passes the search text.")
            ]
            .into();
        }
        for (position, &index) in page.shown.iter().enumerate() {
            let selected = position == page.selected;
            let (title, subtitle) = dmenu_page::entry_text(&page.entries[index], quick_look);
            let row = self.list_row(
                self.initial_badge(&title, selected),
                title,
                subtitle.filter(|_| self.subtitles),
                selected,
            );
            let row: Element<Message> = mouse_area(row)
                .on_press(Message::DmenuSelected(position))
                .into();
            let row: Element<Message> = if selected {
                container(row).id(crate::scroll::ROOT_SELECTION).into()
            } else {
                row
            };
            list = list.push(row);
        }
        scrollable(container(list).padding(Padding::new(6.0).top(8)))
            .id(crate::scroll::ROOT_RESULTS)
            .height(Length::Shrink)
            .into()
    }
}
