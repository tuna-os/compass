//! Snippets in the launcher: Manage Snippets, and the forms that create,
//! edit and fill them in.
//!
//! A child module of `app` so it can reach the launcher's state without
//! widening it; the decisions that do not need that state are in
//! [`crate::snippets_page`].

use compass_core::shortcut_form::Mode;
use iced::keyboard::{Key, Modifiers, key::Named};

use super::{
    Direction, Element, LauncherApp, Length, Message, Padding, Page, PanelSection, PanelState,
    Task, chord_direction, column, container, focus_search, mouse_area, next_selection, scrollable,
};
use crate::action_panel::Action;
use crate::preferences_page::Purpose;
use crate::snippets_page::{self, SnippetsPage, Status};

const COPY: &str = "snippet.copy";
const PASTE: &str = "snippet.paste";
const EDIT: &str = "snippet.edit";
const DUPLICATE: &str = "snippet.duplicate";
const REMOVE: &str = "snippet.remove";

const NEEDS_ENGINE: &str =
    "Snippets need the Compass engine, and this window is running without one";

/// The actions a snippet row offers, as `ManageSnippetsSection` arranges
/// them, with Paste after Copy.
fn panel_sections() -> Vec<PanelSection> {
    vec![PanelSection {
        name: String::new(),
        actions: vec![
            Action::new("Copy to clipboard")
                .with_id(COPY)
                .with_shortcut("enter"),
            Action::new("Paste").with_id(PASTE),
            Action::new("Edit snippet")
                .with_id(EDIT)
                .with_shortcut("Ctrl+E"),
            Action::new("Duplicate snippet")
                .with_id(DUPLICATE)
                .with_shortcut("Ctrl+D"),
            Action::new("Remove snippet")
                .with_id(REMOVE)
                .with_shortcut("Ctrl+X"),
        ],
    }]
}

impl LauncherApp {
    /// Opens Manage Snippets, as it was left when a form was opened over it,
    /// and asks for the list.
    pub(super) fn open_manage_snippets(&mut self) -> Task<Message> {
        let page = self.parked_snippets.take().unwrap_or_default();
        self.page = Page::Snippets(page);
        let Some(backend) = self.backend.clone() else {
            if let Page::Snippets(page) = &mut self.page {
                page.apply(Err(NEEDS_ENGINE.to_owned()));
            }
            return focus_search();
        };
        Task::batch([
            Task::perform(
                async move { backend.list_snippets().await },
                Message::SnippetsLoaded,
            ),
            focus_search(),
        ])
    }

    /// Opens the snippet form: creating, or editing or duplicating the
    /// selected snippet of Manage Snippets.
    pub(super) fn open_snippet_form(&mut self, mode: Mode) -> Task<Message> {
        let (existing, from_manage) = match &self.page {
            Page::Snippets(page) if mode != Mode::Create => {
                let Some(snippet) = page.selected_snippet() else {
                    return Task::none();
                };
                (Some(snippet.clone()), true)
            }
            Page::Snippets(_) => (None, true),
            _ => (None, false),
        };
        self.parked_snippets = match &self.page {
            Page::Snippets(page) if from_manage => Some(page.clone()),
            _ => None,
        };
        self.panel = None;
        self.page = Page::Preferences(Box::new(snippets_page::form(
            mode,
            existing.as_ref(),
            from_manage,
        )));
        iced::widget::operation::focus_next()
    }

    /// Copies or pastes the selected snippet: through its arguments form
    /// when its text takes arguments, at once otherwise.
    fn use_selected_snippet(&mut self, paste: bool) -> Task<Message> {
        let Page::Snippets(page) = &self.page else {
            return Task::none();
        };
        let Some(snippet) = page.selected_snippet().cloned() else {
            return Task::none();
        };
        self.panel = None;
        if let Some(form) = snippets_page::arguments_form(&snippet, paste) {
            self.parked_snippets = match &self.page {
                Page::Snippets(page) => Some(page.clone()),
                _ => None,
            };
            self.page = Page::Preferences(Box::new(form));
            return iced::widget::operation::focus_next();
        }
        self.send_snippet(snippet.id, Vec::new(), paste)
    }

    /// Asks the engine to expand a snippet for copying, or to paste it.
    fn send_snippet(
        &mut self,
        id: String,
        arguments: Vec<(String, String)>,
        paste: bool,
    ) -> Task<Message> {
        let Some(backend) = self.backend.clone() else {
            self.snippet_notice(NEEDS_ENGINE.to_owned());
            return Task::none();
        };
        if paste {
            Task::perform(
                async move { backend.paste_snippet(id, arguments).await },
                Message::SnippetPasted,
            )
        } else {
            Task::perform(
                async move { backend.expand_snippet(id, arguments).await },
                Message::SnippetExpanded,
            )
        }
    }

    /// Removes the selected snippet.
    fn remove_selected_snippet(&mut self) -> Task<Message> {
        let Page::Snippets(page) = &self.page else {
            return Task::none();
        };
        let (Some(snippet), Some(backend)) = (page.selected_snippet(), self.backend.clone()) else {
            return Task::none();
        };
        let id = snippet.id.clone();
        self.panel = None;
        Task::perform(
            async move { backend.remove_snippet(id).await },
            Message::SnippetsLoaded,
        )
    }

    /// Shows why a snippet action did not happen, where the person is.
    fn snippet_notice(&mut self, reason: String) {
        match &mut self.page {
            Page::Snippets(page) => page.notice = Some(reason),
            Page::Preferences(page) => page.notice = Some(reason),
            _ => self.error = Some(reason),
        }
    }

    /// Opens the action panel over the selected snippet, if one is.
    pub(super) fn open_snippet_panel(&mut self) -> Option<Task<Message>> {
        let Page::Snippets(page) = &self.page else {
            return None;
        };
        page.selected_snippet()?;
        self.panel = Some(PanelState::new(panel_sections()));
        Some(iced::widget::operation::focus(super::PANEL_INPUT))
    }

    /// Runs a snippet action from the panel, if `id` is one.
    pub(super) fn snippet_panel_action(&mut self, id: &str) -> Option<Task<Message>> {
        if !matches!(self.page, Page::Snippets(_)) {
            return None;
        }
        let task = match id {
            COPY => self.use_selected_snippet(false),
            PASTE => self.use_selected_snippet(true),
            EDIT => self.open_snippet_form(Mode::Edit),
            DUPLICATE => self.open_snippet_form(Mode::Duplicate),
            REMOVE => self.remove_selected_snippet(),
            _ => return None,
        };
        self.panel = None;
        Some(task)
    }

    /// Manage Snippets' keys: the list's navigation, Enter to copy, Ctrl+E
    /// edit, Ctrl+D duplicate, Ctrl+X remove, Escape to go back.
    pub(super) fn snippets_page_key(&mut self, key: &Key, modifiers: Modifiers) -> Task<Message> {
        if let Key::Character(c) = key.as_ref()
            && modifiers.control()
            && !modifiers.alt()
            && !modifiers.logo()
            && !modifiers.shift()
        {
            match c.to_lowercase().as_str() {
                "e" => return self.open_snippet_form(Mode::Edit),
                "d" => return self.open_snippet_form(Mode::Duplicate),
                "x" => return self.remove_selected_snippet(),
                _ => {}
            }
        }
        let Page::Snippets(page) = &mut self.page else {
            return Task::none();
        };
        let direction = match key.as_ref() {
            Key::Named(Named::ArrowDown) => Some(Direction::Down),
            Key::Named(Named::ArrowUp) => Some(Direction::Up),
            Key::Named(Named::Escape) => return self.update(Message::Back),
            Key::Named(Named::Enter) => return self.use_selected_snippet(false),
            _ => chord_direction(self.keybinding, key.as_ref(), modifiers),
        };
        if let Some(direction) = direction {
            page.selected = next_selection(
                page.shown.len(),
                page.selected,
                direction,
                self.wrap_navigation,
            );
            return crate::scroll::reveal_root_selection();
        }
        Task::none()
    }

    /// Submits a snippet form: its arguments, to copy or paste it, or its
    /// fields, to save it. `None` when the form showing is not a snippet form.
    pub(super) fn submit_snippet_form(&mut self) -> Option<Task<Message>> {
        let Page::Preferences(page) = &mut self.page else {
            return None;
        };
        match page.purpose {
            Purpose::SnippetArguments { paste } => {
                if let Err(missing) = page.submission() {
                    page.notice = Some(format!("Fill in {}", missing.join(", ")));
                    return Some(Task::none());
                }
                let id = page.command_id.clone();
                let arguments = snippets_page::argument_values(page);
                Some(self.send_snippet(id, arguments, paste))
            }
            Purpose::SnippetForm { mode, .. } => {
                let (name, text, keyword, word) = snippets_page::form_values(page);
                // The keyword's applications are not edited here; they stay
                // what the snippet edited or duplicated had.
                let apps = self
                    .parked_snippets
                    .as_ref()
                    .and_then(|parked| parked.all.iter().find(|s| s.id == page.command_id))
                    .and_then(|snippet| snippet.expansion.as_ref())
                    .map(|expansion| expansion.apps.clone())
                    .unwrap_or_default();
                let draft = crate::backend::SnippetDraft {
                    id: (mode == Mode::Edit).then(|| page.command_id.clone()),
                    name,
                    text,
                    keyword: (!keyword.is_empty()).then_some(keyword),
                    word,
                    apps,
                };
                let Some(backend) = self.backend.clone() else {
                    page.notice = Some(NEEDS_ENGINE.to_owned());
                    return Some(Task::none());
                };
                Some(Task::perform(
                    async move { backend.save_snippet(draft).await },
                    Message::SnippetSaved,
                ))
            }
            _ => None,
        }
    }

    /// Where Escape goes from a snippet form: back to Manage Snippets when it
    /// was opened from there.
    pub(super) fn back_from_snippet_form(&mut self) -> Option<Task<Message>> {
        let Page::Preferences(page) = &self.page else {
            return None;
        };
        match page.purpose {
            Purpose::SnippetForm {
                from_manage: true, ..
            }
            | Purpose::SnippetArguments { .. } => Some(self.open_manage_snippets()),
            _ => None,
        }
    }

    /// Handles the snippet messages.
    pub(super) fn snippet_message(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::SnippetsLoaded(result) => {
                if let Page::Snippets(page) = &mut self.page {
                    page.apply(result);
                } else if let Err(reason) = result {
                    self.snippet_notice(reason);
                }
                crate::scroll::reveal_root_selection()
            }
            Message::SnippetSaved(Ok(snippets)) => {
                let from_manage = matches!(
                    &self.page,
                    Page::Preferences(page)
                        if matches!(page.purpose, Purpose::SnippetForm { from_manage: true, .. })
                );
                if from_manage {
                    let task = self.open_manage_snippets();
                    if let Page::Snippets(page) = &mut self.page {
                        page.apply(Ok(snippets));
                    }
                    return task;
                }
                self.parked_snippets = None;
                self.page = Page::Root;
                Task::batch([self.search_task(), focus_search()])
            }
            Message::SnippetSaved(Err(reason))
            | Message::SnippetExpanded(Err(reason))
            | Message::SnippetPasted(Err(reason)) => {
                self.snippet_notice(reason);
                Task::none()
            }
            Message::SnippetExpanded(Ok(text)) => {
                self.parked_snippets = None;
                if matches!(self.page, Page::Preferences(_)) {
                    self.page = Page::Root;
                }
                Task::batch([iced::clipboard::write(text), self.conceal()])
            }
            Message::SnippetPasted(Ok(())) => {
                self.parked_snippets = None;
                if matches!(self.page, Page::Preferences(_)) {
                    self.page = Page::Root;
                }
                self.conceal()
            }
            Message::SnippetsQueryChanged(query) => {
                if let Page::Snippets(page) = &mut self.page {
                    page.query = query;
                    page.notice = None;
                    page.selected = 0;
                    page.refilter();
                }
                crate::scroll::reveal_root_selection()
            }
            Message::SnippetSelected(position) => {
                let Page::Snippets(page) = &mut self.page else {
                    return Task::none();
                };
                if position >= page.shown.len() {
                    return Task::none();
                }
                page.selected = position;
                self.use_selected_snippet(false)
            }
            _ => Task::none(),
        }
    }

    /// Manage Snippets' body.
    pub(super) fn snippets_body<'a>(&'a self, page: &'a SnippetsPage) -> Element<'a, Message> {
        match &page.status {
            Status::Loading => return self.notice("Loading snippets…"),
            Status::Failed(reason) => return self.notice(reason),
            Status::Ready if page.all.is_empty() => {
                return self.notice("No snippets. Create one with Create Snippet.");
            }
            Status::Ready if page.shown.is_empty() => {
                return self.notice("No snippets match");
            }
            Status::Ready => {}
        }
        let mut list = column![].spacing(f32::from(self.geometry.row_spacing));
        for (position, &index) in page.shown.iter().enumerate() {
            let Some(snippet) = page.all.get(index) else {
                continue;
            };
            let selected = position == page.selected;
            let row = self.list_row(
                self.initial_badge(&snippet.name, selected),
                snippet.name.clone(),
                self.subtitles.then(|| snippets_page::subtitle(snippet)),
                selected,
            );
            let row: Element<Message> = mouse_area(row)
                .on_press(Message::SnippetSelected(position))
                .into();
            let row: Element<Message> = if selected {
                container(row).id(crate::scroll::ROOT_SELECTION).into()
            } else {
                row
            };
            list = list.push(row);
        }
        let rows = scrollable(container(list).padding(Padding::new(6.0).top(8)))
            .id(crate::scroll::ROOT_RESULTS)
            .height(Length::Shrink);
        match &page.notice {
            Some(notice) => column![rows, self.notice(notice)].into(),
            None => rows.into(),
        }
    }
}
