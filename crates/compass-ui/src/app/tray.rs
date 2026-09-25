//! Search Tray in the launcher: other applications' tray icons, activating
//! them and browsing their menus.
//!
//! A child module of `app` so it can reach the launcher's state without
//! widening it; the lists' state is in [`crate::tray_page`].

use iced::keyboard::{Key, Modifiers, key::Named};

use super::{
    Direction, Element, LauncherApp, Length, Message, Padding, Page, PanelSection, PanelState,
    Task, chord_direction, column, container, focus_search, mouse_area, next_selection, scrollable,
    text,
};
use crate::action_panel::Action;
use crate::backend::TrayItemRow;
use crate::tray_page::{self, ItemAction, Mode, Status, TrayPage};

const ACTIVATE: &str = "tray.activate";
const BROWSE: &str = "tray.browse";
const SECONDARY: &str = "tray.secondary";
const TRIGGER: &str = "tray.trigger";

const NEEDS_ENGINE: &str =
    "Search Tray needs the Compass engine, and this window is running without one";

impl LauncherApp {
    /// Opens Search Tray and asks for the items.
    pub(super) fn open_search_tray(&mut self) -> Task<Message> {
        self.page = Page::Tray(TrayPage::default());
        Task::batch([self.load_tray_items(), focus_search()])
    }

    fn load_tray_items(&mut self) -> Task<Message> {
        let Some(backend) = self.backend.clone() else {
            if let Page::Tray(page) = &mut self.page {
                page.apply_items(Err(NEEDS_ENGINE.to_owned()));
            }
            return Task::none();
        };
        Task::perform(
            async move { backend.tray_items().await },
            Message::TrayItemsLoaded,
        )
    }

    fn load_tray_menu(&self, key: String) -> Task<Message> {
        let Some(backend) = self.backend.clone() else {
            return Task::none();
        };
        Task::perform(
            async move {
                let result = backend.tray_menu(key.clone()).await;
                (key, result)
            },
            |(key, result)| Message::TrayMenuLoaded { key, result },
        )
    }

    /// Runs `action` on the selected item.
    fn tray_item_action(&mut self, action: ItemAction) -> Task<Message> {
        self.panel = None;
        let Page::Tray(page) = &mut self.page else {
            return Task::none();
        };
        if action == ItemAction::BrowseMenu {
            let Some(key) = page.push_menu() else {
                return Task::none();
            };
            return Task::batch([self.load_tray_menu(key), focus_search()]);
        }
        let (Some(item), Some(backend)) = (page.selected_item(), self.backend.clone()) else {
            return Task::none();
        };
        let key = item.key.clone();
        let secondary = action == ItemAction::SecondaryActivate;
        Task::perform(
            async move { backend.tray_activate(key, secondary).await.map(|()| true) },
            Message::TrayActed,
        )
    }

    /// Clicks the selected menu entry; the launcher closes unless it was a
    /// toggle, whose new state is then read back (`TrayMenuViewHost`).
    fn trigger_tray_entry(&mut self) -> Task<Message> {
        self.panel = None;
        let Page::Tray(page) = &self.page else {
            return Task::none();
        };
        let (Some(key), Some(entry), Some(backend)) =
            (page.menu_key(), page.selected_entry(), self.backend.clone())
        else {
            return Task::none();
        };
        let key = key.to_owned();
        let id = entry.id;
        let toggle = entry.toggled.is_some();
        Task::perform(
            async move { backend.tray_trigger(key, id).await.map(|()| !toggle) },
            Message::TrayActed,
        )
    }

    /// The panel over the selected item or entry.
    pub(super) fn open_tray_panel(&mut self) -> Option<Task<Message>> {
        let Page::Tray(page) = &self.page else {
            return None;
        };
        let actions = if let Some(item) = page.selected_item() {
            tray_page::item_actions(item)
                .into_iter()
                .enumerate()
                .map(|(position, action)| {
                    let id = match action {
                        ItemAction::Activate => ACTIVATE,
                        ItemAction::BrowseMenu => BROWSE,
                        ItemAction::SecondaryActivate => SECONDARY,
                    };
                    let entry = Action::new(action.title()).with_id(id);
                    if position == 0 {
                        entry.with_shortcut("enter")
                    } else {
                        entry
                    }
                })
                .collect()
        } else {
            page.selected_entry()?;
            vec![
                Action::new("Trigger")
                    .with_id(TRIGGER)
                    .with_shortcut("enter"),
            ]
        };
        self.panel = Some(PanelState::new(vec![PanelSection {
            name: String::new(),
            actions,
        }]));
        Some(iced::widget::operation::focus(super::PANEL_INPUT))
    }

    /// Runs a Search Tray panel action, if `id` is one.
    pub(super) fn tray_panel_action(&mut self, id: &str) -> Option<Task<Message>> {
        if !matches!(self.page, Page::Tray(_)) {
            return None;
        }
        let action = match id {
            ACTIVATE => ItemAction::Activate,
            BROWSE => ItemAction::BrowseMenu,
            SECONDARY => ItemAction::SecondaryActivate,
            TRIGGER => return Some(self.trigger_tray_entry()),
            _ => return None,
        };
        Some(self.tray_item_action(action))
    }

    /// Enter: the first of the item's actions, or the entry's trigger.
    fn tray_primary(&mut self) -> Task<Message> {
        let Page::Tray(page) = &self.page else {
            return Task::none();
        };
        if page.menu_key().is_some() {
            return self.trigger_tray_entry();
        }
        let Some(first) = page
            .selected_item()
            .and_then(|item| tray_page::item_actions(item).first().copied())
        else {
            return Task::none();
        };
        self.tray_item_action(first)
    }

    /// The view's keys.
    pub(super) fn tray_page_key(&mut self, key: &Key, modifiers: Modifiers) -> Task<Message> {
        let Page::Tray(page) = &mut self.page else {
            return Task::none();
        };
        let direction = match key.as_ref() {
            Key::Named(Named::ArrowDown) => Some(Direction::Down),
            Key::Named(Named::ArrowUp) => Some(Direction::Up),
            Key::Named(Named::Escape) => {
                if page.pop_menu() {
                    return focus_search();
                }
                return self.update(Message::Back);
            }
            Key::Named(Named::Enter) => return self.tray_primary(),
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

    /// Handles the view's messages.
    pub(super) fn tray_message(&mut self, message: Message) -> Task<Message> {
        let Page::Tray(page) = &mut self.page else {
            return Task::none();
        };
        match message {
            Message::TrayItemsLoaded(result) => {
                page.apply_items(result);
                self.warm_tray_icons();
            }
            Message::TrayMenuLoaded { key, result } => page.apply_menu(&key, result),
            Message::TrayQueryChanged(query) => {
                page.query = query;
                page.notice = None;
                page.refilter();
            }
            Message::TraySelected(position) => {
                if position < page.shown.len() {
                    page.selected = position;
                }
            }
            Message::TrayActed(Ok(true)) => return self.conceal(),
            Message::TrayActed(Ok(false)) => {
                if let Some(key) = page.menu_key().map(str::to_owned) {
                    return self.load_tray_menu(key);
                }
            }
            Message::TrayActed(Err(reason)) => page.notice = Some(reason),
            _ => {}
        }
        crate::scroll::reveal_root_selection()
    }

    /// Resolves the theme icons the items name.
    fn warm_tray_icons(&mut self) {
        if !self.icons {
            return;
        }
        let Page::Tray(page) = &self.page else {
            return;
        };
        let names: Vec<String> = page
            .items
            .iter()
            .filter_map(|item| item.icon_name.clone())
            .collect();
        let find = self.icon_lookup.clone();
        self.icon_cache
            .warm(names.iter().map(String::as_str), &|name| find.find(name));
    }

    /// An item's icon: its file, its pixels, its theme name, else
    /// `AppWindow`.
    fn tray_icon(&self, item: &TrayItemRow, selected: bool) -> Element<'_, Message> {
        let size = Length::Fixed(f32::from(self.geometry.icon_size));
        if !self.icons {
            return self.initial_badge(&item.title, selected);
        }
        if let Some(png) = &item.icon_png {
            let handle = iced::widget::image::Handle::from_bytes(png.clone());
            return container(
                iced::widget::image(handle)
                    .width(Length::Fill)
                    .height(Length::Fill),
            )
            .width(size)
            .height(size)
            .into();
        }
        let art = item
            .icon_path
            .as_deref()
            .and_then(|path| crate::icons::classify(std::path::Path::new(path)))
            .or_else(|| {
                item.icon_name
                    .as_deref()
                    .and_then(|name| self.icon_cache.cached(name))
                    .cloned()
            });
        let glyph = art.map_or_else(
            || crate::icons::Glyph::builtin("app-window"),
            crate::icons::Glyph::Art,
        );
        self.glyph_or_initial(Some(&glyph), &item.title, selected)
    }

    /// The view's body.
    pub(super) fn tray_body<'a>(&'a self, page: &'a TrayPage) -> Element<'a, Message> {
        let in_menu = matches!(page.mode, Mode::Menu { .. });
        let empty = match &page.status {
            Status::Loading if in_menu => Some("Loading the menu…"),
            Status::Loading => Some("Looking for tray items…"),
            Status::Failed(reason) => Some(reason.as_str()),
            Status::Ready if page.shown.is_empty() && page.query.is_empty() && in_menu => {
                Some("This menu is empty")
            }
            Status::Ready if page.shown.is_empty() && page.query.is_empty() => {
                Some("No tray items")
            }
            Status::Ready if page.shown.is_empty() => Some("No matches"),
            Status::Ready => None,
        };
        if let Some(empty) = empty {
            return match &page.notice {
                Some(notice) => column![self.notice(empty), self.notice(notice)].into(),
                None => self.notice(empty),
            };
        }
        let palette = self.palette();
        let mut list = column![].spacing(f32::from(self.geometry.row_spacing));
        if let Mode::Menu { title, .. } = &page.mode {
            list = list.push(self.section_heading(title.clone()));
        }
        for (position, &index) in page.shown.iter().enumerate() {
            let selected = position == page.selected;
            let item = if in_menu {
                let entry = &page.entries[index];
                let mark = entry.toggled.and_then(|on| {
                    let glyph = if on {
                        crate::icons::default_mark()
                    } else {
                        crate::icons::Glyph::builtin("circle")
                    };
                    self.glyph(
                        &glyph,
                        selected,
                        f32::from(self.geometry.subtitle_size) + 4.0,
                    )
                });
                let icon = entry
                    .icon_name
                    .as_deref()
                    .and_then(|name| self.icon_cache.cached(name))
                    .cloned()
                    .map(crate::icons::Glyph::Art);
                self.list_row_parts(
                    self.glyph_or_initial(icon.as_ref(), &entry.label, selected),
                    entry.label.clone(),
                    None,
                    mark,
                    selected,
                )
            } else {
                let row = &page.items[index];
                let attention = row.attention.then(|| {
                    text(tray_page::ATTENTION)
                        .font(self.font())
                        .size(f32::from(self.geometry.subtitle_size))
                        .color(
                            crate::icons::tile_color(
                                compass_core::commands::Tile::Orange,
                                palette.accent,
                            )
                            .to_iced(),
                        )
                        .into()
                });
                self.list_row_parts(
                    self.tray_icon(row, selected),
                    row.title.clone(),
                    self.subtitles
                        .then(|| row.subtitle.clone())
                        .filter(|line| !line.is_empty()),
                    attention,
                    selected,
                )
            };
            let item: Element<Message> = mouse_area(item)
                .on_press(Message::TraySelected(position))
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
}
