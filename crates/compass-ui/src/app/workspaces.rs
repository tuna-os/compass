//! The window-management commands beyond Switch Windows: Switch Workspaces
//! (`SwitchWorkspacesViewHost`) and the fullscreen, floating and overview
//! toggles (`wm-extension.cpp`), offered where the compositor can do them.

use iced::keyboard::{Key, Modifiers, key::Named};

use super::{
    Direction, Element, LauncherApp, Length, Message, Padding, Page, PanelSection, PanelState,
    Task, chord_direction, column, container, focus_search, mouse_area, next_selection, scrollable,
    text,
};
use crate::action_panel::Action;
use crate::backend::{WORKSPACES_NEED_ENGINE, WindowToggle};
use crate::workspaces_page::{self, Status, WorkspacesPage};

const SWITCH: &str = "workspace.switch";

impl LauncherApp {
    /// Asks the engine what the window manager can do, so root search offers
    /// only the commands it can run.
    pub(super) fn window_capabilities_task(&self) -> Task<Message> {
        let Some(windows) = self.windows.clone() else {
            return Task::none();
        };
        Task::perform(
            async move { windows.window_manager_capabilities().await },
            Message::WindowCapabilities,
        )
    }

    /// Opens Switch Workspaces and asks for the workspaces.
    pub(super) fn open_switch_workspaces(&mut self) -> Task<Message> {
        self.page = Page::Workspaces(WorkspacesPage::default());
        Task::batch([self.list_workspaces_task(), focus_search()])
    }

    fn list_workspaces_task(&mut self) -> Task<Message> {
        let Page::Workspaces(page) = &mut self.page else {
            return Task::none();
        };
        let Some(windows) = self.windows.clone() else {
            page.apply(Err(WORKSPACES_NEED_ENGINE.to_owned()));
            return Task::none();
        };
        Task::perform(
            async move { windows.list_workspaces().await },
            Message::WorkspacesLoaded,
        )
    }

    /// Runs a toggle, as `ToggleFullscreenWindowCommand` and its siblings:
    /// the launcher goes once it is done, and stays to say why it was not.
    pub(super) fn run_window_toggle(&mut self, toggle: WindowToggle) -> Task<Message> {
        let Some(windows) = self.windows.clone() else {
            self.error = Some(WORKSPACES_NEED_ENGINE.to_owned());
            return Task::none();
        };
        Task::perform(
            async move { windows.toggle_window_state(toggle).await },
            Message::WindowToggled,
        )
    }

    fn switch_to_selected_workspace(&mut self) -> Task<Message> {
        self.panel = None;
        let Page::Workspaces(page) = &self.page else {
            return Task::none();
        };
        let (Some(row), Some(windows)) = (page.selected_row(), self.windows.clone()) else {
            return Task::none();
        };
        let id = row.id.clone();
        Task::perform(
            async move { windows.focus_workspace(id).await },
            Message::WorkspaceFocused,
        )
    }

    /// The panel over the selected workspace: its one action.
    pub(super) fn open_workspaces_panel(&mut self) -> Option<Task<Message>> {
        let Page::Workspaces(page) = &self.page else {
            return None;
        };
        page.selected_row()?;
        self.panel = Some(PanelState::new(vec![PanelSection {
            name: String::new(),
            actions: vec![
                Action::new(compass_core::window_switcher::switch_to_workspace_label(
                    false,
                ))
                .with_id(SWITCH)
                .with_shortcut("enter"),
            ],
        }]));
        Some(iced::widget::operation::focus(super::PANEL_INPUT))
    }

    /// Runs a Switch Workspaces panel action, if `id` is one.
    pub(super) fn workspaces_panel_action(&mut self, id: &str) -> Option<Task<Message>> {
        if id != SWITCH || !matches!(self.page, Page::Workspaces(_)) {
            return None;
        }
        Some(self.switch_to_selected_workspace())
    }

    /// The view's keys.
    pub(super) fn workspaces_page_key(&mut self, key: &Key, modifiers: Modifiers) -> Task<Message> {
        let Page::Workspaces(page) = &mut self.page else {
            return Task::none();
        };
        let direction = match key.as_ref() {
            Key::Named(Named::ArrowDown) => Some(Direction::Down),
            Key::Named(Named::ArrowUp) => Some(Direction::Up),
            Key::Named(Named::Escape) => return self.update(Message::Back),
            Key::Named(Named::Enter) => return self.switch_to_selected_workspace(),
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

    /// Handles the window-management messages.
    pub(super) fn workspaces_message(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::WindowCapabilities(Ok(caps)) => {
                if caps != self.app_index.window_capabilities() {
                    self.app_index.set_window_capabilities(caps);
                    if matches!(self.page, Page::Root) {
                        return self.search_task();
                    }
                }
                Task::none()
            }
            Message::WindowCapabilities(Err(error)) => {
                tracing::debug!(%error, "no window manager capabilities");
                Task::none()
            }
            Message::WorkspacesQueryChanged(query) => {
                if let Page::Workspaces(page) = &mut self.page {
                    page.query = query;
                    page.notice = None;
                    page.refilter();
                }
                crate::scroll::reveal_root_selection()
            }
            Message::WorkspacesLoaded(result) => {
                if let Page::Workspaces(page) = &mut self.page {
                    page.apply(result);
                }
                Task::none()
            }
            Message::WorkspaceSelected(position) => {
                if let Page::Workspaces(page) = &mut self.page
                    && position < page.shown.len()
                {
                    page.selected = position;
                }
                self.switch_to_selected_workspace()
            }
            Message::WorkspaceFocused(Ok(())) | Message::WindowToggled(Ok(())) => self.conceal(),
            Message::WorkspaceFocused(Err(reason)) => {
                if let Page::Workspaces(page) = &mut self.page {
                    page.notice = Some(reason);
                }
                Task::none()
            }
            Message::WindowToggled(Err(reason)) => {
                self.error = Some(reason);
                Task::none()
            }
            _ => Task::none(),
        }
    }

    /// The view's body.
    pub(super) fn workspaces_body<'a>(&'a self, page: &'a WorkspacesPage) -> Element<'a, Message> {
        match &page.status {
            Status::Loading => return self.notice("Loading workspaces…"),
            Status::Failed(reason) => return self.notice(reason),
            Status::Ready if page.all.is_empty() => return self.notice("No workspaces"),
            Status::Ready if page.shown.is_empty() => {
                return self.notice("No workspaces match");
            }
            Status::Ready => {}
        }
        let mut list = column![
            container(text(workspaces_page::SECTION).font(self.font()).size(12))
                .padding(Padding::new(4.0).left(10))
        ]
        .spacing(f32::from(self.geometry.row_spacing));
        for (position, &index) in page.shown.iter().enumerate() {
            let Some(workspace) = page.all.get(index) else {
                continue;
            };
            let selected = position == page.selected;
            let apps: Vec<&str> = workspace
                .apps
                .iter()
                .map(|(name, _)| name.as_str())
                .collect();
            let row = self.list_row_with(
                self.initial_badge(&workspace.name, selected),
                workspace.name.clone(),
                self.subtitles.then(|| workspaces_page::subtitle(workspace)),
                (!apps.is_empty()).then(|| apps.join(", ")),
                selected,
            );
            let row: Element<Message> = mouse_area(row)
                .on_press(Message::WorkspaceSelected(position))
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
