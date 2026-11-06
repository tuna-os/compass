//! Switch Windows in the launcher: the engine's list of open windows,
//! focusing the selected one and closing it.
//!
//! The compositor-specific work is the engine's; this module draws the list
//! and turns Enter and `Ctrl+Q` into requests for it.

use super::{
    Element, LauncherApp, Length, Message, Padding, Page, Task, a11y, column, container,
    mouse_area, scrollable,
};

impl LauncherApp {
    /// Switch Windows' messages: its query, the list, and the engine's
    /// answers to focusing and closing.
    pub(super) fn windows_message(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::WindowsQueryChanged(query) => {
                if let Page::Windows(page) = &mut self.page {
                    page.query = query;
                    page.notice = None;
                    page.refilter();
                }
                crate::scroll::reveal_root_selection()
            }
            Message::WindowsLoaded(result) => {
                if let Page::Windows(page) = &mut self.page {
                    page.apply(result, std::process::id());
                }
                self.warm_window_icons();
                crate::scroll::reveal_root_selection()
            }
            Message::WindowSelected(position) => {
                if let Page::Windows(page) = &mut self.page
                    && position < page.shown.len()
                {
                    page.selected = position;
                }
                self.activate_selected_window()
            }
            Message::WindowActivated(result) => match result {
                // The window the user picked is in front now; get out of the way.
                Ok(()) => self.conceal(),
                Err(reason) => {
                    if let Page::Windows(page) = &mut self.page {
                        page.notice = Some(reason);
                    }
                    Task::none()
                }
            },
            Message::ShellWindowClosed(result) => {
                if let (Err(reason), Page::Windows(page)) = (&result, &mut self.page) {
                    page.notice = Some(reason.clone());
                }
                self.list_windows_task()
            }
            _ => Task::none(),
        }
    }

    pub(super) fn windows_body<'a>(
        &'a self,
        page: &'a crate::windows_page::WindowsPage,
    ) -> Element<'a, Message> {
        let geometry = self.geometry;
        if let Some(empty) = page.empty_notice() {
            return self.notice(empty);
        }
        let mut list = column![].spacing(f32::from(geometry.row_spacing));
        for (position, &index) in page.shown.iter().enumerate() {
            let Some(window) = page.all.get(index) else {
                continue;
            };
            let selected = position == page.selected;
            let title = crate::windows_page::title(window).to_owned();
            let row = self.list_row(
                self.window_icon(window, selected),
                title,
                self.subtitles.then(|| window.app.clone()),
                selected,
            );
            let row: Element<Message> = mouse_area(row)
                .on_press(Message::WindowSelected(position))
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

    /// The window switcher's list, for assistive technology.
    pub(super) fn windows_semantics(
        &self,
        page: &crate::windows_page::WindowsPage,
    ) -> a11y::PageSemantics {
        let items = page
            .shown
            .iter()
            .map(|&index| {
                page.all
                    .get(index)
                    .map_or_else(crate::semantics::Item::default, |window| {
                        a11y::item(crate::windows_page::title(window), Some(window.app.clone()))
                    })
            })
            .collect();
        a11y::PageSemantics::list("Windows", items, page.selected, page.empty_notice())
            .with_status(page.notice.as_deref(), &page.query)
    }

    /// Asks the engine for the open windows.
    pub(super) fn list_windows_task(&mut self) -> Task<Message> {
        let Page::Windows(page) = &mut self.page else {
            return Task::none();
        };
        page.notice = None;
        let Some(windows) = self.windows.clone() else {
            page.apply(
                Err(
                    "Window switching needs the Compass engine, and this window is running \
                     without one"
                        .to_owned(),
                ),
                std::process::id(),
            );
            return Task::none();
        };
        Task::perform(
            async move { windows.list_windows().await },
            Message::WindowsLoaded,
        )
    }

    /// Switches to the selected window.
    pub(super) fn activate_selected_window(&mut self) -> Task<Message> {
        let Page::Windows(page) = &self.page else {
            return Task::none();
        };
        let (Some(row), Some(windows)) = (page.selected_row(), self.windows.clone()) else {
            return Task::none();
        };
        let id = row.id;
        Task::perform(
            async move { windows.activate_window(id).await },
            Message::WindowActivated,
        )
    }

    /// Closes the selected window, when it can be closed.
    pub(super) fn close_selected_window(&mut self) -> Task<Message> {
        let Page::Windows(page) = &mut self.page else {
            return Task::none();
        };
        let Some(row) = page.selected_row() else {
            return Task::none();
        };
        if !row.can_close {
            page.notice = Some(format!("{} cannot be closed", row.title));
            return Task::none();
        }
        let Some(windows) = self.windows.clone() else {
            return Task::none();
        };
        let id = row.id;
        Task::perform(
            async move { windows.close_window(id).await },
            Message::ShellWindowClosed,
        )
    }
}
