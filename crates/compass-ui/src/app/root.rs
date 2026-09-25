//! The root search's own behaviour beyond ranking (`RootViewHost`,
//! `RootSearchModel`, `RootSearchActionGenerator`): the favourites above the
//! suggestions, the row's action panel, the alias form, the space-bar alias
//! shortcut, the up arrow's search history and the status bar's clock.

use compass_core::root_items::RootEdit;
use compass_core::root_view;

use super::{
    Confirm, ConfirmAction, Direction, LauncherApp, Message, Page, PanelSection, PanelState,
    ProviderScope, RootRow, Task, focus_search,
};
use crate::action_panel::Action;
use crate::backend::{PreferenceInput, PreferenceInputKind};
use crate::preferences_page::{FieldValue, PreferencesPage, Purpose};
use crate::shortcut_recorder::{Outcome as RecorderOutcome, ShortcutRecorder};

const OPEN: &str = "root.open";
const COPY_DEEPLINK: &str = "root.copy-deeplink";
const RESET_RANKING: &str = "root.reset-ranking";
const FAVORITE: &str = "root.favorite";
const UNFAVORITE: &str = "root.unfavorite";
const FAVORITE_DOWN: &str = "root.favorite-down";
const FAVORITE_UP: &str = "root.favorite-up";
const ALIAS: &str = "root.alias";
const COPY_ID: &str = "root.copy-id";
const SHORTCUT: &str = "root.shortcut";
const DISABLE: &str = "root.disable";

/// The heading over the favourites.
pub(super) const FAVORITES_HEADING: &str = "Favorites";

/// The heading over the rest of the empty query's rows.
pub(super) const SUGGESTIONS_HEADING: &str = "Suggestions";

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| {
            i64::try_from(elapsed.as_secs()).unwrap_or(i64::MAX)
        })
}

impl LauncherApp {
    /// The `provider:entrypoint` id of a root row, for the rows that are
    /// root items.
    pub(super) fn root_id(&self, row: RootRow) -> Option<String> {
        use compass_core::root_items::entrypoint_id;
        match row {
            RootRow::App(index) => {
                let item = self.app_index.items().get(index)?;
                Some(entrypoint_id(
                    compass_core::root_items::APPS_PROVIDER_ID,
                    &compass_core::root_items::app_entrypoint_id(item.key()),
                ))
            }
            RootRow::Command(command) => Some(command.id()),
            RootRow::Extension(index) => Some(self.app_index.extensions().get(index)?.id.clone()),
            RootRow::Shortcut(index) => Some(entrypoint_id(
                compass_core::shortcut::SHORTCUTS_PROVIDER_ID,
                &self.app_index.shortcuts().get(index)?.id,
            )),
            RootRow::Script(index) => Some(entrypoint_id(
                compass_core::script_scan::SCRIPTS_PROVIDER_ID,
                &self.app_index.scripts().get(index)?.id,
            )),
            RootRow::RhaiScript(index) => {
                Some(self.app_index.rhai_scripts().get(index)?.entrypoint_id())
            }
            RootRow::Calculator | RootRow::Fallback(_) => None,
        }
    }

    /// The root row an entrypoint id names, if this window knows it.
    fn row_for_id(&self, id: &str) -> Option<RootRow> {
        if let Some(command) = compass_core::commands::by_id(id) {
            return Some(RootRow::Command(command));
        }
        if let Some(index) = self
            .app_index
            .extensions()
            .iter()
            .position(|command| command.id == id)
        {
            return Some(RootRow::Extension(index));
        }
        if let Some(shortcut) = self.app_index.shortcut_by_entrypoint(id) {
            return self
                .app_index
                .shortcuts()
                .iter()
                .position(|known| known.id == shortcut.id)
                .map(RootRow::Shortcut);
        }
        if let Some(script) = self.app_index.script_by_entrypoint(id) {
            return self
                .app_index
                .scripts()
                .iter()
                .position(|known| known.id == script.id)
                .map(RootRow::Script);
        }
        if let Some(script) = self.app_index.rhai_script_by_entrypoint(id) {
            return self
                .app_index
                .rhai_scripts()
                .iter()
                .position(|known| known.id == script.id)
                .map(RootRow::RhaiScript);
        }
        let position = self.app_index.position_by_entrypoint(id)?;
        (!self.app_index.items()[position].is_action()).then_some(RootRow::App(position))
    }

    /// For the empty query, puts the favourites first, in the order they
    /// were arranged, and takes them out of the suggestions below
    /// (`queryFavorites`, and the search that leaves favourites out). A
    /// favourite that is disabled or unknown here is not shown.
    pub(super) fn apply_favorites(&mut self) {
        self.favorites_len = 0;
        if !self.query.is_empty() || !matches!(self.page, Page::Root) {
            return;
        }
        let favorites: Vec<RootRow> = self
            .root_config
            .favorites
            .iter()
            .filter(|id| self.app_index.root(id).is_none_or(|root| root.meta.enabled))
            .filter_map(|id| self.row_for_id(id))
            .collect();
        self.results.retain(|row| !favorites.contains(row));
        self.favorites_len = favorites.len();
        self.results.splice(0..0, favorites);
    }

    /// The heading drawn above the root row at `position`, if a section
    /// starts there: only the empty query has sections, and only when it
    /// has favourites.
    pub(super) fn root_heading_at(&self, position: usize) -> Option<&'static str> {
        if self.favorites_len == 0 || !self.query.is_empty() {
            return None;
        }
        if position == 0 {
            return Some(FAVORITES_HEADING);
        }
        (position == self.favorites_len).then_some(SUGGESTIONS_HEADING)
    }

    /// The panel over the selected root row: its own actions (an
    /// application's, or opening), then the item actions every root item has
    /// (`RootSearchActionGenerator::generateActions`).
    pub(super) fn open_root_panel(&mut self) -> Option<Task<Message>> {
        if !matches!(self.page, Page::Root) {
            return None;
        }
        let row = self.selected_row()?;
        let id = self.root_id(row)?;
        let mut sections = match (row, self.selected_item()) {
            (RootRow::App(_), Some(item)) => super::actions_for_app(item),
            _ => vec![PanelSection {
                name: String::new(),
                actions: vec![
                    Action::new(match row {
                        RootRow::Command(_) | RootRow::Extension(_) => "Open command",
                        _ => "Open",
                    })
                    .with_id(OPEN)
                    .with_shortcut("enter"),
                ],
            }],
        };
        let favorite = self.root_config.favorites.iter().position(|fav| *fav == id);
        let mut item = vec![
            Action::new("Copy Deeplink")
                .with_id(COPY_DEEPLINK)
                .with_shortcut("ctrl+shift+c"),
            Action::new("Reset ranking").with_id(RESET_RANKING),
            match favorite {
                Some(_) => Action::new("Remove from favorites").with_id(UNFAVORITE),
                None => Action::new("Add to favorites").with_id(FAVORITE),
            },
        ];
        if let Some(at) = favorite {
            if at + 1 < self.root_config.favorites.len() {
                item.push(
                    Action::new("Move down in favorites")
                        .with_id(FAVORITE_DOWN)
                        .with_shortcut("ctrl+shift+down"),
                );
            }
            if at != 0 {
                item.push(
                    Action::new("Move up in favorites")
                        .with_id(FAVORITE_UP)
                        .with_shortcut("ctrl+shift+up"),
                );
            }
        }
        item.push(Action::new("Set alias").with_id(ALIAS));
        item.push(Action::new("Set Global Shortcut").with_id(SHORTCUT));
        item.push(Action::new("Copy ID").with_id(COPY_ID));
        item.push(
            Action::new("Disable item")
                .with_id(DISABLE)
                .with_shortcut("ctrl+x"),
        );
        sections.push(PanelSection {
            name: String::new(),
            actions: item,
        });
        self.panel = Some(PanelState::new(sections));
        let focus = iced::widget::operation::focus(super::PANEL_INPUT);
        // An application's panel grows its running-only actions once the
        // engine says it runs (`AppRootItem::newActionPanel`).
        if let (RootRow::App(_), Some(item)) = (row, self.selected_item()) {
            let (key, desktop_id) = (item.key().to_owned(), item.desktop_id().to_owned());
            self.app_runtime = None;
            return Some(Task::batch([focus, self.app_runtime_task(key, desktop_id)]));
        }
        Some(focus)
    }

    /// Runs a root item action, if `action` is one.
    pub(super) fn root_panel_action(&mut self, action: &str) -> Option<Task<Message>> {
        if !matches!(self.page, Page::Root) {
            return None;
        }
        let row = self.selected_row()?;
        let id = self.root_id(row)?;
        let task = match action {
            OPEN => {
                self.panel = None;
                return Some(self.update(Message::LaunchSelected));
            }
            COPY_DEEPLINK => {
                let link = compass_core::root_items::deeplink(&id)?;
                Task::batch([iced::clipboard::write(link), focus_search()])
            }
            COPY_ID => Task::batch([iced::clipboard::write(id), focus_search()]),
            RESET_RANKING => {
                self.confirm = Some(Confirm {
                    title: "Are you sure?".to_owned(),
                    message: "You will have to rebuild search history for this item in order \
                              for it to reappear on top of the root search results."
                        .to_owned(),
                    confirm_text: "Reset".to_owned(),
                    action: ConfirmAction::RootEdit(id, RootEdit::ResetRanking),
                });
                Task::none()
            }
            DISABLE => {
                self.confirm = Some(Confirm {
                    title: "Are you sure?".to_owned(),
                    message: "You will need to go in the settings to manually re-enable it."
                        .to_owned(),
                    confirm_text: "Disable".to_owned(),
                    action: ConfirmAction::RootEdit(id, RootEdit::Disable),
                });
                Task::none()
            }
            FAVORITE => self.edit_root_item(id, RootEdit::Favorite(true)),
            UNFAVORITE => self.edit_root_item(id, RootEdit::Favorite(false)),
            FAVORITE_DOWN => self.edit_root_item(id, RootEdit::MoveFavorite { down: true }),
            FAVORITE_UP => self.edit_root_item(id, RootEdit::MoveFavorite { down: false }),
            ALIAS => {
                self.panel = None;
                return Some(self.open_alias_form(row, id));
            }
            SHORTCUT => {
                let title = self.root_title(row).unwrap_or_default();
                let current = self
                    .app_index
                    .root(&id)
                    .and_then(|root| root.meta.shortcut.clone());
                if let Some(panel) = self.panel.as_mut() {
                    panel.recorder = Some(ShortcutRecorder::new(id, title, current));
                }
                return Some(Task::none());
            }
            _ => return None,
        };
        self.panel = None;
        Some(task)
    }

    /// A key event while the shortcut recorder shows: an accepted chord is
    /// kept for the item and closes the panel, Escape goes back to the
    /// actions (`SetRootItemShortcutAction`'s accept handler, `setShortcut`).
    pub(super) fn recorder_event(&mut self, event: &iced::keyboard::Event) -> Task<Message> {
        let bound: Vec<(String, String, String)> = self
            .app_index
            .roots()
            .iter()
            .filter_map(|root| {
                let shortcut = root.meta.shortcut.clone()?;
                Some((root.id.clone(), root.title.clone(), shortcut))
            })
            .collect();
        let Some(recorder) = self
            .panel
            .as_mut()
            .and_then(|panel| panel.recorder.as_mut())
        else {
            return Task::none();
        };
        let outcome = recorder.key(
            event,
            bound
                .iter()
                .map(|(id, title, shortcut)| (id.as_str(), title.as_str(), shortcut.as_str())),
        );
        match outcome {
            RecorderOutcome::Recording => Task::none(),
            RecorderOutcome::Back => {
                if let Some(panel) = self.panel.as_mut() {
                    panel.recorder = None;
                }
                iced::widget::operation::focus(super::PANEL_INPUT)
            }
            RecorderOutcome::Save(shortcut) => {
                let id = recorder.id.clone();
                self.panel = None;
                self.edit_root_item(id, RootEdit::Shortcut(shortcut))
            }
        }
    }

    /// Applies `edit` to this window's root settings at once, and asks the
    /// engine to keep it; the list is searched again when it has.
    pub(super) fn edit_root_item(&mut self, id: String, edit: RootEdit) -> Task<Message> {
        if compass_core::root_items::apply_edit(&mut self.root_config, &id, &edit) {
            self.app_index.apply_root_config(&self.root_config);
        }
        match self.backend.clone() {
            Some(backend) => Task::batch([
                Task::perform(
                    async move { backend.edit_root_item(id, edit).await },
                    Message::RootItemEdited,
                ),
                focus_search(),
            ]),
            None => Task::batch([self.search_task(), focus_search()]),
        }
    }

    /// The engine's answer to a root edit: search again, or say why not.
    pub(super) fn root_item_edited(&mut self, result: Result<(), String>) -> Task<Message> {
        if let Err(reason) = result {
            self.error = Some(reason);
            return Task::none();
        }
        if matches!(self.page, Page::Root) {
            let selected = self.selected;
            let task = self.search_task();
            self.selected = selected;
            return task;
        }
        Task::none()
    }

    /// The alias form (`AliasFormViewHost`): one field, holding the alias
    /// the item has.
    fn open_alias_form(&mut self, row: RootRow, id: String) -> Task<Message> {
        let existing = self
            .app_index
            .root(&id)
            .and_then(|root| root.meta.alias.clone());
        let title = self.root_title(row).unwrap_or_default();
        let field = PreferenceInput {
            name: "alias".to_owned(),
            title: "Alias".to_owned(),
            description: String::new(),
            placeholder: String::new(),
            required: false,
            kind: PreferenceInputKind::Text,
            value: Some(serde_json::Value::String(
                root_view::alias_form_initial_value(existing.as_deref()),
            )),
        };
        self.page = Page::Preferences(Box::new(PreferencesPage::new(
            Purpose::Alias,
            id,
            root_view::alias_form_title(&title),
            vec![field],
        )));
        iced::widget::operation::focus_next()
    }

    /// A root row's title, for the alias form's.
    fn root_title(&self, row: RootRow) -> Option<String> {
        Some(match row {
            RootRow::App(index) => self.app_index.items().get(index)?.name().to_owned(),
            RootRow::Command(command) => command.title.to_owned(),
            RootRow::Fallback(fallback) => return self.fallback_title(fallback),
            RootRow::Extension(index) => self.app_index.extensions().get(index)?.title.clone(),
            RootRow::Shortcut(index) => {
                crate::shortcuts_page::display_name(self.app_index.shortcuts().get(index)?)
                    .to_owned()
            }
            RootRow::Script(index) => self.app_index.scripts().get(index)?.title.clone(),
            RootRow::RhaiScript(index) => self.app_index.rhai_scripts().get(index)?.title.clone(),
            RootRow::Calculator => return None,
        })
    }

    /// Saves the alias form and goes back to the root. `None` when the form
    /// showing is not the alias form.
    pub(super) fn submit_alias_form(&mut self) -> Option<Task<Message>> {
        let Page::Preferences(form) = &self.page else {
            return None;
        };
        if form.purpose != Purpose::Alias {
            return None;
        }
        let id = form.command_id.clone();
        let alias = match form.values.first() {
            Some(FieldValue::Text(text)) => text.clone(),
            _ => String::new(),
        };
        self.page = Page::Root;
        Some(self.edit_root_item(id, RootEdit::Alias(alias)))
    }

    /// Escape from the alias form: back to the root, unchanged.
    pub(super) fn back_from_alias_form(&mut self) -> Option<Task<Message>> {
        let Page::Preferences(form) = &self.page else {
            return None;
        };
        if form.purpose != Purpose::Alias {
            return None;
        }
        self.page = Page::Root;
        Some(focus_search())
    }

    /// Whether the row can be opened by its alias and a space
    /// (`supportsAliasSpaceShortcut`): a command that opens a view.
    fn supports_alias_space(&self, row: RootRow) -> bool {
        match row {
            RootRow::Command(command) => compass_core::commands::opens_a_view(command.kind),
            RootRow::Extension(index) => self
                .app_index
                .extensions()
                .get(index)
                .is_some_and(|command| command.mode == compass_core::manifest::CommandMode::View),
            _ => false,
        }
    }

    /// Whether the row takes arguments, which the C++ asks for in the search
    /// bar's completer and this launcher in an arguments form.
    fn has_completer(&self, row: RootRow) -> bool {
        let index = &self.app_index;
        match row {
            RootRow::Shortcut(at) => index
                .shortcuts()
                .get(at)
                .is_some_and(|shortcut| !shortcut.link.arguments.is_empty()),
            RootRow::Extension(at) => index
                .extensions()
                .get(at)
                .is_some_and(|command| !command.arguments.is_empty()),
            RootRow::Script(at) => index
                .scripts()
                .get(at)
                .is_some_and(|script| !script.arguments.is_empty()),
            _ => false,
        }
    }

    /// The space-bar alias shortcut (`tryAliasFastTrack`): a space typed
    /// after exactly the selected item's alias opens it rather than being
    /// typed. An item that takes arguments opens its arguments form, where
    /// the C++ focuses the completer's first field (nothing has been typed
    /// into it yet, as the form is not open). `None` lets the text through.
    pub(super) fn alias_space(&mut self, typed: &str) -> Option<Task<Message>> {
        if !matches!(self.page, Page::Root) || self.panel.is_some() {
            return None;
        }
        if typed.strip_suffix(' ') != Some(self.query.as_str()) {
            return None;
        }
        let row = self.selected_row()?;
        let id = self.root_id(row)?;
        let alias = self.app_index.root(&id)?.meta.alias.clone();
        let outcome = root_view::space_outcome(
            &self.query,
            alias.as_deref(),
            self.has_completer(row),
            true,
            self.supports_alias_space(row),
        );
        matches!(
            outcome,
            root_view::SpaceOutcome::Activate | root_view::SpaceOutcome::FocusCompleter
        )
        .then(|| self.update(Message::LaunchSelected))
    }

    /// Remembers the search an action ran from (`beforeActionExecuted`).
    pub(super) fn record_search(&mut self) {
        let now = u64::try_from(now_secs()).unwrap_or_default();
        if !self.search_history.add(&self.query, now) {
            return;
        }
        if let Some(path) = &self.search_history_path
            && let Err(err) = self.search_history.save_file(path)
        {
            tracing::warn!(%err, "could not save the search history");
        }
    }

    /// The up arrow at the top of the list reaches back through past
    /// searches (`inputFilter`), unless navigation wraps. `None` when the
    /// arrow should move the selection instead.
    pub(super) fn history_up(&mut self, direction: Option<Direction>) -> Option<Task<Message>> {
        if direction != Some(Direction::Up)
            || !matches!(self.page, Page::Root)
            || self.panel.is_some()
        {
            return None;
        }
        let selected = i32::try_from(self.selected).unwrap_or(i32::MAX);
        if !root_view::up_cycles_history(self.wrap_navigation, selected, 0) {
            return None;
        }
        let offset = root_view::next_history_offset(self.history_offset);
        let queries = self.search_history.queries();
        match root_view::history_entry_at(&queries, offset, &self.query) {
            Some((reached, query)) => {
                self.history_offset = Some(reached);
                self.query = query;
                self.error = None;
                Some(self.search_task())
            }
            None => {
                self.history_offset = Some(offset);
                Some(Task::none())
            }
        }
    }

    /// A `vicinae://launch/...` deeplink the engine handed over: a
    /// provider's search view, or an item launched with the link's text.
    pub(super) fn open_launch_link(
        &mut self,
        link: compass_core::root_items::LaunchLink,
    ) -> Task<Message> {
        use compass_core::root_items::LaunchTarget;
        let index = &self.app_index;
        match link.target(|id| index.has_provider(id)) {
            Ok(LaunchTarget::Provider(id)) => self.open_provider_search(&id, link.fallback_text),
            Ok(LaunchTarget::Entrypoint(id)) => {
                let Some(backend) = self.backend.clone() else {
                    return Task::none();
                };
                let query = link.fallback_text;
                Task::perform(
                    async move { backend.launch_command(id, query).await },
                    Message::BuiltinCommandDone,
                )
            }
            Err(reason) => {
                self.error = Some(reason);
                Task::none()
            }
        }
    }

    /// Opens the provider search view (`ProviderSearchViewHost`): root search
    /// over the items of `id` alone, every one of them for the empty query,
    /// titled `Search <provider>`, with `text` typed in. Leaving it closes
    /// the window, as the deeplink's `setInstantDismiss` does.
    pub(super) fn open_provider_search(&mut self, id: &str, text: Option<String>) -> Task<Message> {
        let Some(title) = self.app_index.provider_title(id) else {
            self.error = Some(format!("No provider has the id {id}"));
            return Task::none();
        };
        let closing = self.close_extension_view();
        self.panel = None;
        self.page = Page::Root;
        self.history_offset = None;
        self.provider_scope = Some(ProviderScope {
            id: id.to_owned(),
            placeholder: format!("Search {title}"),
            title,
        });
        self.query = text.unwrap_or_default();
        Task::batch([closing, self.search_task(), focus_search()])
    }

    /// A second passed: redraws the clock when it is due
    /// (`scheduleNextClockTick`), on a multiple of its interval.
    pub(super) fn clock_tick(&mut self) {
        let Some(clock) = &self.clock else {
            self.clock_text = None;
            return;
        };
        let now = now_secs();
        if now < self.clock_next_at {
            return;
        }
        let interval = i64::try_from(clock.interval).unwrap_or(60);
        self.clock_text = Some(clock_text(&clock.format, now));
        self.clock_next_at = now + root_view::next_clock_tick_secs(now, interval);
    }
}

/// The clock's text at `now` (seconds since the epoch), in `format`, local
/// time.
#[must_use]
pub(super) fn clock_text(format: &str, now: i64) -> String {
    let Ok(at) = jiff::Timestamp::from_second(now) else {
        return String::new();
    };
    let at = at.to_zoned(jiff::tz::TimeZone::system());
    let small = |value: i8| u8::try_from(value).unwrap_or_default();
    let broken = compass_core::qt_date::DateTime {
        year: i32::from(at.year()),
        month: small(at.month()),
        day: small(at.day()),
        hour: small(at.hour()),
        minute: small(at.minute()),
        second: small(at.second()),
        millisecond: u16::try_from(at.millisecond()).unwrap_or_default(),
        weekday: small(at.weekday().to_monday_one_offset()),
        zone: at.strftime("%Z").to_string(),
    };
    compass_core::qt_date::format(format, &broken)
}
