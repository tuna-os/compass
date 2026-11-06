//! Search Files in the launcher: the engine's recent files, then its
//! search once typing has paused, the category filter, the selected file's
//! preview, and opening or revealing it.
//!
//! The panel's file actions are `file_actions`'s; this module draws the
//! list and keeps its answers in order against the query.

use super::panel::PanelState;
use super::{
    Alignment, Element, LauncherApp, Length, Message, Padding, Page, Task, a11y, column, container,
    file_actions, focus_search, mouse_area, preview, row, scrollable, text,
};

impl LauncherApp {
    /// Search Files' messages: its query, its answers and opening a file.
    pub(super) fn files_message(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::FilesQueryChanged(query) => {
                if let Page::Files(page) = &mut self.page {
                    page.set_query(query);
                }
                self.files_query_task()
            }
            Message::FilesDebounced(generation) => self.files_search_task(generation),
            Message::FilesCategoryChanged(key) => {
                let Page::Files(page) = &mut self.page else {
                    return Task::none();
                };
                let generation = page.set_category(&key);
                self.view
                    .view_memory
                    .set(crate::view_memory::FILE_CATEGORY, &key);
                Task::batch([self.files_search_task(generation), focus_search()])
            }
            Message::FilesLoaded { generation, result } => {
                if let Page::Files(page) = &mut self.page {
                    page.apply(generation, result);
                }
                self.warm_file_icons();
                crate::scroll::reveal_root_selection()
            }
            Message::FilesSelected(position) => {
                if let Page::Files(page) = &mut self.page
                    && position < page.rows.len()
                {
                    page.selected = position;
                    page.refresh_preview();
                }
                self.open_selected_file(false)
            }
            Message::FileOpened(Ok(())) => self.conceal(),
            Message::FileOpened(Err(reason)) => {
                if let Page::Files(page) = &mut self.page {
                    page.notice = Some(reason);
                }
                Task::none()
            }
            _ => Task::none(),
        }
    }

    /// Search Files' body: its state, or its heading and rows.
    pub(super) fn files_body<'a>(
        &'a self,
        page: &'a crate::files_page::FilesPage,
    ) -> Element<'a, Message> {
        let geometry = self.geometry;
        let key = page
            .category
            .clone()
            .unwrap_or_else(|| compass_core::file_search::CATEGORY_FILTER_KEYS[0].to_owned());
        let filter = container(
            iced::widget::pick_list(
                compass_core::file_search::CATEGORY_FILTER_KEYS
                    .iter()
                    .map(|key| (*key).to_owned())
                    .collect::<Vec<_>>(),
                Some(key),
                Message::FilesCategoryChanged,
            )
            .text_size(12)
            .font(self.font())
            .style({
                let palette = self.palette();
                move |_, status| crate::design::dropdown(palette, status)
            })
            .menu_style({
                let palette = self.palette();
                move |_| crate::design::dropdown_menu(palette)
            }),
        )
        .width(Length::Fill)
        .align_x(Alignment::End)
        .padding(Padding::new(4.0).right(10));
        // The loading indicator (`setLoading`): a query is out.
        let indicator = container(
            text(page.indicator().unwrap_or_default())
                .font(self.font())
                .size(12)
                .color(self.palette().muted.to_iced()),
        )
        .padding(Padding::new(8.0).left(12));
        let filter = row![indicator, filter];
        if let Some(empty) = page.empty_notice() {
            return column![filter, self.notice(empty)].into();
        }
        let home =
            compass_core::xdg_dirs::home_dir().map(|home| home.to_string_lossy().into_owned());
        let heading = text(page.heading.clone())
            .font(self.font())
            .size(12)
            .color(self.palette().muted.to_iced());
        let mut list = column![container(heading).padding(Padding::new(4.0).left(10))]
            .spacing(f32::from(geometry.row_spacing));
        for (position, file) in page.rows.iter().enumerate() {
            let selected = position == page.selected;
            let subtitle = self
                .subtitles
                .then(|| crate::files_page::subtitle(file, home.as_deref()));
            let row = self.list_row(
                self.glyph_or_initial(
                    self.icons
                        .then(|| self.file_glyphs.cached(&file.path))
                        .flatten(),
                    &file.name,
                    selected,
                ),
                file.name.clone(),
                subtitle,
                selected,
            );
            let row: Element<Message> = mouse_area(row)
                .on_press(Message::FilesSelected(position))
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
        let rows: Element<Message> = match &page.preview {
            Some(preview) => row![
                container(rows).width(Length::FillPortion(preview::LIST_PORTION)),
                self.file_preview_pane(preview, true),
            ]
            .into(),
            None => rows.into(),
        };
        match &page.notice {
            Some(notice) => column![filter, rows, self.notice(notice)].into(),
            None => column![filter, rows].into(),
        }
    }

    /// Search Files' list and the selected file's preview, for assistive
    /// technology.
    pub(super) fn files_semantics(
        &self,
        page: &crate::files_page::FilesPage,
    ) -> a11y::PageSemantics {
        let home =
            compass_core::xdg_dirs::home_dir().map(|home| home.to_string_lossy().into_owned());
        let items = page
            .rows
            .iter()
            .map(|file| {
                a11y::item(
                    file.name.clone(),
                    Some(crate::files_page::subtitle(file, home.as_deref())),
                )
            })
            .collect();
        let label = if page.heading.is_empty() {
            "Files"
        } else {
            page.heading.as_str()
        };
        let notice = page.notice.as_deref().or(page.indicator());
        a11y::PageSemantics::list(label, items, page.selected, page.empty_notice())
            .with_status(notice, &page.query)
            .with_detail(
                page.preview
                    .as_ref()
                    .map(|preview| a11y::file_preview_document(preview, true)),
            )
    }

    /// Search Files' panel for the selected file, without the engine's
    /// answer about it: the panel itself waits for that answer before it
    /// opens, but a shortcut only needs to know which action it names.
    pub(super) fn files_panel_now(&self) -> Option<PanelState> {
        let Page::Files(page) = &self.page else {
            return None;
        };
        let path = &page.selected_row()?.path;
        let assumed = crate::backend::FileActions {
            mime: self.file_mime.clone(),
            has_opener: true,
            can_set_wallpaper: true,
            can_paste: true,
        };
        Some(PanelState::new(file_actions::file_panel_sections(
            path, &assumed,
        )))
    }

    /// Opens Search Files with `query` typed, and the remembered category
    /// (`restoreCategoryFilter`: anything but "All").
    pub(super) fn open_search_files(&mut self, query: String) -> Task<Message> {
        let mut page = crate::files_page::FilesPage::default();
        if let Some(key) = self.view.view_memory.get(crate::view_memory::FILE_CATEGORY) {
            page.set_category(key);
        }
        page.set_query(query);
        self.page = Page::Files(page);
        Task::batch([self.files_query_task(), focus_search()])
    }

    /// Asks Search Files' query: at once, or once its debounce runs out.
    pub(super) fn files_query_task(&mut self) -> Task<Message> {
        let Page::Files(page) = &self.page else {
            return Task::none();
        };
        let generation = page.generation;
        match crate::files_page::debounce_for(&page.query) {
            Some(delay) => Task::perform(
                async move {
                    tokio::time::sleep(delay).await;
                    generation
                },
                Message::FilesDebounced,
            ),
            None => self.files_search_task(generation),
        }
    }

    /// Sends Search Files' query `generation`, unless the text moved on.
    pub(super) fn files_search_task(&mut self, generation: u64) -> Task<Message> {
        let Page::Files(page) = &mut self.page else {
            return Task::none();
        };
        if page.generation != generation {
            return Task::none();
        }
        let Some(backend) = self.backend.clone() else {
            page.apply(
                generation,
                Err(
                    "Search Files needs the Compass engine, and this window is running \
                     without one"
                        .to_owned(),
                ),
            );
            return Task::none();
        };
        let query = page.query.clone();
        let category = page.category.clone();
        Task::perform(
            async move { backend.search_files(query, category).await },
            move |result| Message::FilesLoaded { generation, result },
        )
    }

    /// Opens the selected file with its default application, or shows it in
    /// the file browser when `reveal`.
    pub(super) fn open_selected_file(&mut self, reveal: bool) -> Task<Message> {
        let Page::Files(page) = &self.page else {
            return Task::none();
        };
        let (Some(row), Some(backend)) = (page.selected_row(), self.backend.clone()) else {
            return Task::none();
        };
        let path = row.path.clone();
        Task::perform(
            async move { backend.open_file(path, reveal).await },
            Message::FileOpened,
        )
    }
}
