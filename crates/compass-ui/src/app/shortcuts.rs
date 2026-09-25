//! Shortcuts (quicklinks) in the launcher: their root rows, Manage Shortcuts,
//! and the forms that create, edit and open them.
//!
//! A child module of `app` so it can reach the launcher's state without
//! widening it; the decisions that do not need that state are in
//! [`crate::shortcuts_page`].

use compass_core::shortcut_form::Mode;
use iced::keyboard::{Key, Modifiers, key::Named};

use super::{
    Direction, Element, LauncherApp, Length, Message, Padding, Page, PanelSection, PanelState,
    RootRow, Task, chord_direction, column, container, focus_search, mouse_area, next_selection,
    scrollable,
};
use crate::action_panel::Action;
use crate::shortcuts_page::{self, ShortcutsPage};

const OPEN: &str = "shortcut.open";
const COPY: &str = "shortcut.copy";
const EDIT: &str = "shortcut.edit";
const DUPLICATE: &str = "shortcut.duplicate";
const REMOVE: &str = "shortcut.remove";

/// The actions a shortcut row offers, as `RootShortcutItem::newActionPanel`
/// arranges them (less "Open with…", which is not ported).
pub(super) fn panel_sections(in_manage: bool) -> Vec<PanelSection> {
    vec![
        PanelSection {
            name: String::new(),
            actions: vec![
                Action::new("Open shortcut")
                    .with_id(OPEN)
                    .with_shortcut("enter"),
                Action::new("Copy shortcut").with_id(COPY),
            ],
        },
        PanelSection {
            name: String::new(),
            actions: vec![
                Action::new("Edit shortcut")
                    .with_id(EDIT)
                    .with_shortcut("Ctrl+E"),
                Action::new("Duplicate link")
                    .with_id(DUPLICATE)
                    .with_shortcut("Ctrl+D"),
            ],
        },
        PanelSection {
            name: String::new(),
            actions: vec![
                Action::new("Remove link")
                    .with_id(REMOVE)
                    .with_shortcut(if in_manage { "Ctrl+X" } else { "Ctrl+Shift+X" }),
            ],
        },
    ]
}

impl LauncherApp {
    /// Asks the engine for the shortcut list, so root search and Manage
    /// Shortcuts show what the store holds now.
    pub(super) fn refresh_shortcuts_task(&self) -> Task<Message> {
        let Some(backend) = self.backend.clone() else {
            return Task::none();
        };
        Task::perform(
            async move { backend.list_shortcuts().await },
            Message::ShortcutsLoaded,
        )
    }

    /// The shortcut the selection is on: a root row, or a Manage Shortcuts
    /// row. By position in the index's list.
    pub(super) fn selected_shortcut(&self) -> Option<usize> {
        match &self.page {
            Page::Shortcuts(page) => page.selected_index(),
            Page::Root => match self.selected_row()? {
                RootRow::Shortcut(index) => Some(index),
                _ => None,
            },
            _ => None,
        }
    }

    /// Shows why a shortcut action did not happen, where the person is.
    fn shortcut_notice(&mut self, reason: String) {
        match &mut self.page {
            Page::Shortcuts(page) => page.notice = Some(reason),
            Page::Preferences(page) => page.notice = Some(reason),
            _ => self.error = Some(reason),
        }
    }

    /// Takes a new list: root search and Manage Shortcuts both follow it.
    fn apply_shortcuts(&mut self, shortcuts: Vec<crate::backend::Shortcut>) {
        self.app_index.set_shortcuts(
            shortcuts
                .iter()
                .map(compass_core::shortcut_service::from_serialized)
                .collect(),
        );
        if let Page::Shortcuts(page) = &mut self.page {
            page.refilter(self.app_index.shortcuts());
        }
    }

    /// Opens Manage Shortcuts, as it was left when a form was opened over it.
    pub(super) fn open_manage_shortcuts(&mut self) -> Task<Message> {
        let mut page = self.parked_shortcuts.take().unwrap_or_default();
        page.refilter(self.app_index.shortcuts());
        self.page = Page::Shortcuts(page);
        Task::batch([self.refresh_shortcuts_task(), focus_search()])
    }

    /// Opens the shortcut form: creating, or editing or duplicating the
    /// shortcut at `index`.
    pub(super) fn open_shortcut_form(
        &mut self,
        mode: Mode,
        index: Option<usize>,
        from_manage: bool,
    ) -> Task<Message> {
        let mut applications: Vec<(String, String)> = self
            .app_index
            .applications()
            .map(|item| (item.display_name(), item.desktop_id().to_owned()))
            .collect();
        applications.sort_by_key(|(name, _)| name.to_lowercase());
        let existing = index.and_then(|index| self.app_index.shortcuts().get(index));
        self.panel = None;
        self.parked_shortcuts = match &self.page {
            Page::Shortcuts(page) if from_manage => Some(page.clone()),
            _ => None,
        };
        self.page = Page::Preferences(Box::new(shortcuts_page::form(
            mode,
            existing,
            &applications,
            from_manage,
        )));
        iced::widget::operation::focus_next()
    }

    /// Opens the shortcut at `index`: through its arguments form when its
    /// link takes arguments, at once otherwise.
    pub(super) fn open_shortcut_at(&mut self, index: usize) -> Task<Message> {
        let Some(shortcut) = self.app_index.shortcuts().get(index) else {
            return Task::none();
        };
        self.panel = None;
        if let Some(form) = shortcuts_page::arguments_form(shortcut) {
            self.page = Page::Preferences(Box::new(form));
            return iced::widget::operation::focus_next();
        }
        let id = shortcut.id.clone();
        self.send_open_shortcut(id, Vec::new())
    }

    /// Asks the engine to open a shortcut, and counts the use in root search.
    pub(super) fn send_open_shortcut(
        &mut self,
        id: String,
        arguments: Vec<String>,
    ) -> Task<Message> {
        let Some(backend) = self.backend.clone() else {
            self.shortcut_notice(
                "Shortcuts need the Compass engine, and this window is running without one"
                    .to_owned(),
            );
            return Task::none();
        };
        let key = compass_core::root_items::entrypoint_id(
            compass_core::shortcut::SHORTCUTS_PROVIDER_ID,
            &id,
        );
        Task::perform(
            async move {
                backend.open_shortcut(id, arguments).await?;
                if let Err(error) = backend.record_launch(key).await {
                    tracing::warn!(%error, "could not record opening a shortcut");
                }
                Ok(())
            },
            Message::ShortcutOpened,
        )
    }

    /// Copies the shortcut at `index`, expanded, as `CopyShortcutAction` does.
    fn copy_shortcut_at(&mut self, index: usize) -> Task<Message> {
        let (Some(shortcut), Some(backend)) =
            (self.app_index.shortcuts().get(index), self.backend.clone())
        else {
            return Task::none();
        };
        let id = shortcut.id.clone();
        self.panel = None;
        Task::perform(
            async move { backend.expand_shortcut(id, Vec::new()).await },
            Message::ShortcutExpanded,
        )
    }

    /// Removes the shortcut at `index`.
    fn remove_shortcut_at(&mut self, index: usize) -> Task<Message> {
        let (Some(shortcut), Some(backend)) =
            (self.app_index.shortcuts().get(index), self.backend.clone())
        else {
            return Task::none();
        };
        let id = shortcut.id.clone();
        self.panel = None;
        Task::perform(
            async move { backend.remove_shortcut(id).await },
            Message::ShortcutRemoved,
        )
    }

    /// Runs a shortcut action from the panel, if `id` is one.
    pub(super) fn shortcut_panel_action(&mut self, id: &str) -> Option<Task<Message>> {
        let index = self.selected_shortcut()?;
        let from_manage = matches!(self.page, Page::Shortcuts(_));
        let task = match id {
            OPEN => self.open_shortcut_at(index),
            COPY => self.copy_shortcut_at(index),
            EDIT => self.open_shortcut_form(Mode::Edit, Some(index), from_manage),
            DUPLICATE => self.open_shortcut_form(Mode::Duplicate, Some(index), from_manage),
            REMOVE => self.remove_shortcut_at(index),
            _ => return None,
        };
        self.panel = None;
        Some(task)
    }

    /// Opens the action panel over the selected shortcut, if one is.
    pub(super) fn open_shortcut_panel(&mut self) -> Option<Task<Message>> {
        self.selected_shortcut()?;
        let in_manage = matches!(self.page, Page::Shortcuts(_));
        self.panel = Some(PanelState::new(panel_sections(in_manage)));
        Some(iced::widget::operation::focus(super::PANEL_INPUT))
    }

    /// The keys a shortcut row takes without opening the panel: Ctrl+E edit,
    /// Ctrl+D duplicate, and remove — Ctrl+X in Manage Shortcuts
    /// (`action.remove`), Ctrl+Shift+X in root search
    /// (`action.dangerous-remove`), as the C++ binds them.
    pub(super) fn shortcut_chord(
        &mut self,
        key: &Key,
        modifiers: Modifiers,
    ) -> Option<Task<Message>> {
        let Key::Character(c) = key.as_ref() else {
            return None;
        };
        if !modifiers.control() || modifiers.alt() || modifiers.logo() {
            return None;
        }
        let index = self.selected_shortcut()?;
        let in_manage = matches!(self.page, Page::Shortcuts(_));
        let c = c.to_lowercase();
        match (c.as_str(), modifiers.shift()) {
            ("e", false) => Some(self.open_shortcut_form(Mode::Edit, Some(index), in_manage)),
            ("d", false) => Some(self.open_shortcut_form(Mode::Duplicate, Some(index), in_manage)),
            ("x", shift) if shift != in_manage => Some(self.remove_shortcut_at(index)),
            _ => None,
        }
    }

    /// Manage Shortcuts' keys: the list's navigation, Enter to open, Escape
    /// to go back.
    pub(super) fn shortcuts_page_key(&mut self, key: &Key, modifiers: Modifiers) -> Task<Message> {
        let Page::Shortcuts(page) = &mut self.page else {
            return Task::none();
        };
        let direction = match key.as_ref() {
            Key::Named(Named::ArrowDown) => Some(Direction::Down),
            Key::Named(Named::ArrowUp) => Some(Direction::Up),
            Key::Named(Named::Escape) => return self.update(Message::Back),
            Key::Named(Named::Enter) => {
                return match page.selected_index() {
                    Some(index) => self.open_shortcut_at(index),
                    None => Task::none(),
                };
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
            return crate::scroll::reveal_root_selection();
        }
        Task::none()
    }

    /// Submits a shortcut form: its arguments, to open it, or its fields, to
    /// save it. `None` when the form showing is not a shortcut form.
    pub(super) fn submit_shortcut_form(&mut self) -> Option<Task<Message>> {
        use crate::preferences_page::Purpose;
        let Page::Preferences(page) = &mut self.page else {
            return None;
        };
        match page.purpose {
            Purpose::ShortcutArguments => {
                if let Err(missing) = page.submission() {
                    page.notice = Some(format!("Fill in {}", missing.join(", ")));
                    return Some(Task::none());
                }
                let id = page.command_id.clone();
                let arguments = shortcuts_page::argument_values(page);
                Some(self.send_open_shortcut(id, arguments))
            }
            Purpose::ShortcutForm { mode, .. } => {
                if let Err(missing) = page.submission() {
                    page.notice = Some(format!(
                        "{}: fill in {}",
                        compass_core::shortcut_form::VALIDATION_FAILED,
                        missing.join(", ")
                    ));
                    return Some(Task::none());
                }
                let (name, url, app, icon) = shortcuts_page::form_values(page);
                let draft = crate::backend::ShortcutDraft {
                    id: (mode == Mode::Edit).then(|| page.command_id.clone()),
                    name,
                    icon,
                    url,
                    app,
                };
                let Some(backend) = self.backend.clone() else {
                    page.notice = Some(
                        "Shortcuts need the Compass engine, and this window is running without one"
                            .to_owned(),
                    );
                    return Some(Task::none());
                };
                Some(Task::perform(
                    async move { backend.save_shortcut(draft).await },
                    Message::ShortcutSaved,
                ))
            }
            _ => None,
        }
    }

    /// Where Escape goes from a shortcut form: back to Manage Shortcuts when
    /// it was opened from there.
    pub(super) fn back_from_shortcut_form(&mut self) -> Option<Task<Message>> {
        use crate::preferences_page::Purpose;
        let Page::Preferences(page) = &self.page else {
            return None;
        };
        let Purpose::ShortcutForm {
            from_manage: true, ..
        } = page.purpose
        else {
            return None;
        };
        Some(self.open_manage_shortcuts())
    }

    /// Handles the shortcut messages.
    pub(super) fn shortcut_message(&mut self, message: Message) -> Task<Message> {
        use crate::preferences_page::Purpose;
        match message {
            Message::ShortcutsLoaded(Ok(shortcuts)) => {
                self.apply_shortcuts(shortcuts);
                Task::none()
            }
            Message::ShortcutsLoaded(Err(reason)) => {
                tracing::debug!(%reason, "could not list shortcuts");
                Task::none()
            }
            Message::ShortcutSaved(Ok(shortcuts)) => {
                self.apply_shortcuts(shortcuts);
                let from_manage = matches!(
                    &self.page,
                    Page::Preferences(page)
                        if matches!(page.purpose, Purpose::ShortcutForm { from_manage: true, .. })
                );
                if from_manage {
                    return self.open_manage_shortcuts();
                }
                self.page = Page::Root;
                self.error = None;
                Task::batch([self.search_task(), focus_search()])
            }
            Message::ShortcutSaved(Err(reason)) | Message::ShortcutRemoved(Err(reason)) => {
                self.shortcut_notice(reason);
                Task::none()
            }
            Message::ShortcutRemoved(Ok(shortcuts)) => {
                self.apply_shortcuts(shortcuts);
                if matches!(self.page, Page::Root) {
                    return self.search_task();
                }
                Task::none()
            }
            Message::ShortcutOpened(Ok(())) => {
                if matches!(self.page, Page::Preferences(_)) {
                    self.page = Page::Root;
                }
                // `OpenShortcutAction::setClearSearch(true)` from root search.
                self.query.clear();
                self.results.clear();
                Task::batch([self.refresh_shortcuts_task(), self.conceal()])
            }
            Message::ShortcutOpened(Err(reason)) => {
                self.shortcut_notice(reason);
                Task::none()
            }
            Message::ShortcutExpanded(Ok(text)) => {
                Task::batch([iced::clipboard::write(text), self.conceal()])
            }
            Message::ShortcutExpanded(Err(reason)) => {
                self.shortcut_notice(reason);
                Task::none()
            }
            Message::ShortcutsQueryChanged(query) => {
                if let Page::Shortcuts(page) = &mut self.page {
                    page.query = query;
                    page.notice = None;
                    page.selected = 0;
                    page.refilter(self.app_index.shortcuts());
                }
                crate::scroll::reveal_root_selection()
            }
            Message::ShortcutSelected(position) => {
                let Page::Shortcuts(page) = &mut self.page else {
                    return Task::none();
                };
                if position >= page.shown.len() {
                    return Task::none();
                }
                page.selected = position;
                match page.selected_index() {
                    Some(index) => self.open_shortcut_at(index),
                    None => Task::none(),
                }
            }
            _ => Task::none(),
        }
    }

    /// Manage Shortcuts' body.
    pub(super) fn shortcuts_body<'a>(&'a self, page: &'a ShortcutsPage) -> Element<'a, Message> {
        if page.shown.is_empty() {
            let message = if self.app_index.shortcuts().is_empty() {
                "No shortcuts yet. Create one with Create Shortcut."
            } else {
                "No shortcuts match"
            };
            return match &page.notice {
                Some(notice) => column![self.notice(message), self.notice(notice)].into(),
                None => self.notice(message),
            };
        }
        let mut list = column![].spacing(f32::from(self.geometry.row_spacing));
        for (position, &index) in page.shown.iter().enumerate() {
            let Some(shortcut) = self.app_index.shortcuts().get(index) else {
                continue;
            };
            let selected = position == page.selected;
            let title = shortcuts_page::display_name(shortcut);
            let row = self.list_row(
                self.initial_badge(title, selected),
                title.to_owned(),
                self.subtitles.then(|| shortcut.link.raw.clone()),
                selected,
            );
            let row: Element<Message> = mouse_area(row)
                .on_press(Message::ShortcutSelected(position))
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
