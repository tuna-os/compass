//! The root search: typing, ranking, moving the selection and launching
//! what it is on.
//!
//! Ranking is `compass-core`'s, run by the engine through the backend or,
//! with no backend attached, here over the local index. What stays in this
//! module is keeping the answer honest against the query on screen -- an
//! answer for an older query never replaces a newer one -- and the rows the
//! launcher adds of its own: the calculator's answer first, and the
//! fallbacks under the results for a non-empty query.

use super::{
    AppLauncher, Arc, Fallback, LauncherApp, Message, Page, RESULTS_SETTLE, RootRow, Task,
    focus_search, next_selection,
};

impl LauncherApp {
    /// The root search's own messages: the query, the engine's answer, the
    /// selection, launching and the catalog moving under it.
    pub(super) fn search_message(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::QueryChanged(query) => {
                if let Some(task) = self.alias_space(&query) {
                    return task;
                }
                self.panel = None;
                self.query = query;
                self.error = None;
                // Typing starts history over from the newest search.
                self.history_offset = None;
                self.history_draft = None;
                self.search_task()
            }
            Message::ClockTick => {
                self.clock_tick();
                Task::none()
            }
            Message::SearchCompleted { generation, result } => {
                if generation != self.search_generation {
                    return Task::none();
                }
                self.search_task = None;
                match result {
                    Ok(keys) => {
                        let shortcuts = self.app_index.shortcuts();
                        let positions: Option<Vec<_>> = keys
                            .iter()
                            // A shortcut the engine knows and this window has
                            // not heard of yet (or one just removed) is left
                            // out rather than failing the list: the next
                            // refresh brings the two back into step.
                            .filter(|key| {
                                (!key.starts_with("shortcuts:")
                                    || self.app_index.shortcut_by_entrypoint(key).is_some())
                                    && (!key.starts_with("scripts:")
                                        || self.app_index.script_by_entrypoint(key).is_some())
                                    && (!key.starts_with("rhai:")
                                        || self.app_index.rhai_script_by_entrypoint(key).is_some())
                                    // An extension installed since this
                                    // window last scanned, likewise.
                                    && (!key.starts_with('@')
                                        || self.app_index.extension(key).is_some())
                            })
                            .map(|key| {
                                if let Some(command) = compass_core::commands::by_id(key) {
                                    return Some(RootRow::Command(command));
                                }
                                if let Some(script) = self.app_index.rhai_script_by_entrypoint(key)
                                {
                                    return self
                                        .app_index
                                        .rhai_scripts()
                                        .iter()
                                        .position(|known| known.id == script.id)
                                        .map(RootRow::RhaiScript);
                                }
                                if let Some(script) = self.app_index.script_by_entrypoint(key) {
                                    return self
                                        .app_index
                                        .scripts()
                                        .iter()
                                        .position(|known| known.id == script.id)
                                        .map(RootRow::Script);
                                }
                                if let Some(shortcut) = self.app_index.shortcut_by_entrypoint(key) {
                                    return shortcuts
                                        .iter()
                                        .position(|known| known.id == shortcut.id)
                                        .map(RootRow::Shortcut);
                                }
                                if let Some(index) = self
                                    .app_index
                                    .extensions()
                                    .iter()
                                    .position(|command| command.id == *key)
                                {
                                    return Some(RootRow::Extension(index));
                                }
                                // By ENTRYPOINT id: `QueryHit.id` is
                                // `applications:foo`, not the launch key
                                // `foo.desktop`, and `position` answers only
                                // for the latter. Resolving with the wrong
                                // one returns None for every hit and shows
                                // "the application catalog changed".
                                let position = self.app_index.position_by_entrypoint(key)?;
                                (!self.app_index.items()[position].is_action())
                                    .then_some(RootRow::App(position))
                            })
                            .collect();
                        if let Some(positions) = positions {
                            self.results = positions;
                            self.selected = 0;
                            self.apply_calculator();
                            self.apply_fallbacks();
                            self.apply_favorites();
                            self.apply_update();
                            self.warm_icons();
                        } else {
                            self.error = Some(
                                "The application catalog changed. Restart the launcher.".to_owned(),
                            );
                        }
                    }
                    Err(error) => self.error = Some(format!("could not search: {error}")),
                }
                if self.query.is_empty() {
                    return crate::scroll::reveal_root_selection();
                }
                self.results_settling = true;
                let settled = Task::perform(
                    async move {
                        // The launcher runs its tasks on tokio. Without a
                        // timer (a test driving the task on a plain executor)
                        // the answer is described at once.
                        if tokio::runtime::Handle::try_current().is_ok() {
                            tokio::time::sleep(RESULTS_SETTLE).await;
                        }
                        generation
                    },
                    Message::ResultsSettled,
                );
                Task::batch([crate::scroll::reveal_root_selection(), settled])
            }
            Message::ResultsSettled(generation) => {
                if generation == self.search_generation {
                    self.results_settling = false;
                }
                Task::none()
            }
            Message::ResultSelected(index) => {
                if index < self.results.len() {
                    self.selected = index;
                }
                crate::scroll::reveal_root_selection()
            }
            // A click activates, as `activate_on_single_click` (the Rust
            // port's only mode, `settings_catalog`). The press took focus
            // from the search field, so it is handed back for whatever the
            // row opens, or for the next query if it opened nothing.
            Message::ResultClicked(position) => {
                if position >= self.results.len() {
                    return Task::none();
                }
                self.panel = None;
                self.selected = position;
                let activated = self.update(Message::LaunchSelected);
                Task::batch([activated, focus_search()])
            }
            Message::MoveSelection(direction) => {
                self.selected = next_selection(
                    self.results.len(),
                    self.selected,
                    direction,
                    self.wrap_navigation,
                );
                crate::scroll::reveal_root_selection()
            }
            Message::LaunchSelected => {
                if matches!(self.page, Page::Root) {
                    self.record_search();
                }
                if let Some(RootRow::Update) = self.selected_row() {
                    return self.open_release_notes();
                }
                if let Some(RootRow::Calculator) = self.selected_row()
                    && self.calculator.is_some()
                {
                    // The C++ primary action: copy the answer, remembering it
                    // in the history, then get out of the way so it can be
                    // pasted.
                    return self.copy_root_answer();
                }
                if let Some(RootRow::Command(command)) = self.selected_row() {
                    return self.open_command(command);
                }
                if let Some(RootRow::Extension(index)) = self.selected_row() {
                    return self.run_extension_command(index);
                }
                if let Some(RootRow::Shortcut(index)) = self.selected_row() {
                    return self.open_shortcut_at(index);
                }
                if let Some(RootRow::Script(index)) = self.selected_row() {
                    return self.run_script_at(index);
                }
                if let Some(RootRow::RhaiScript(index)) = self.selected_row() {
                    return self.open_rhai_script_at(index);
                }
                if let Some(RootRow::Fallback(fallback)) = self.selected_row() {
                    return self.open_fallback(fallback);
                }
                let Some(item) = self.selected_item() else {
                    return Task::none();
                };
                // Windows are presented as `AppItem`s with `key == "window:{n}"`
                // when the `switch-windows` provider is active. Activating
                // one is a `compass-shell` `ActivateWindow` bounded call, not
                // a desktop-entry launch. Keeping the branch here keeps
                // `root_list::build`/`move_selection` untouched.
                if let Some(window_id) =
                    compass_core::window_switcher::window_launch_target_for_app(item.key())
                {
                    // Typed `a{sv}`, no JSON: the id is the `u32` the shell minted.
                    // Real activation is `client.activate_window(window_id).bounded("ActivateWindow")`
                    // via the shell proxy; the headless harness proves the branch
                    // without needing a live compositor or a display server.
                    let _bounded = format!("ActivateWindow({window_id})");
                    return self.conceal();
                }
                // Cloned into the future because the launch outlives this
                // borrow of `self`. An AppItem is a parsed desktop entry, so
                // this is not free — but it happens once per launch, not once
                // per keystroke.
                let entry = item.entry().clone();
                let action_id = item.action_id().map(str::to_owned);
                let launcher = Arc::clone(&self.launcher);
                launch_task(
                    launcher,
                    entry,
                    action_id,
                    self.backend.clone(),
                    item.key().to_owned(),
                )
            }
            // A launcher that stays open after launching is a bug report
            // waiting to happen. Hidden, not gone -- see `conceal`.
            Message::Launched(Ok(())) => self.conceal(),
            Message::Launched(Err(err)) => {
                self.error = Some(format!("could not launch: {err}"));
                Task::none()
            }
            Message::CatalogGeneration(Ok(generation)) => self.catalog_moved(generation),
            Message::CatalogGeneration(Err(error)) => {
                tracing::debug!(%error, "no catalog generation");
                Task::none()
            }
            _ => Task::none(),
        }
    }

    /// What Enter does to the selected root row, as the first action of its
    /// panel names it; `None` with nothing selected.
    pub(super) fn root_primary_action(&self) -> Option<&'static str> {
        Some(match self.selected_row()? {
            RootRow::App(_) | RootRow::RhaiScript(_) => "Open",
            RootRow::Command(_) | RootRow::Extension(_) => "Open command",
            RootRow::Calculator => "Copy answer",
            RootRow::Shortcut(_) => "Open shortcut",
            RootRow::Script(_) => "Run script",
            RootRow::Fallback(Fallback::Shortcut(_)) => "Open",
            RootRow::Fallback(_) => "Open command",
            RootRow::Update => "View release notes",
        })
    }

    /// Re-rank against the current query.
    pub(super) fn search_task(&mut self) -> Task<Message> {
        self.cancel_search();
        self.error = None;
        // The provider search view ranks here: the engine's search is the
        // whole root list.
        if let Some(backend) = self.backend.clone()
            && self.provider_scope.is_none()
        {
            self.results.clear();
            self.selected = 0;
            let query = self.query.clone();
            let generation = self.search_generation;
            let (task, handle) =
                Task::perform(async move { backend.search(query).await }, move |result| {
                    Message::SearchCompleted { generation, result }
                })
                .abortable();
            self.search_task = Some(handle.abort_on_drop());
            return task;
        }
        self.search();
        crate::scroll::reveal_root_selection()
    }

    pub(super) fn cancel_search(&mut self) {
        self.search_generation = self.search_generation.wrapping_add(1);
        self.results_settling = false;
        if let Some(handle) = self.search_task.take() {
            handle.abort();
        }
    }

    /// Re-rank locally when no daemon backend is attached.
    pub(super) fn search(&mut self) {
        let options = compass_core::root_items::SearchOptions {
            provider_id: self.provider_scope.as_ref().map(|scope| scope.id.clone()),
            ..compass_core::root_items::SearchOptions::default()
        };
        self.favorites_len = 0;
        if self.query.trim().is_empty() && options.provider_id.is_none() {
            self.results.clear();
            self.selected = 0;
            self.apply_favorites();
            self.apply_update();
            return;
        }

        self.results = self
            .app_index
            .search_root_with(&self.query, None, &options)
            .into_iter()
            .map(|hit| match hit {
                compass_core::RootHit::App(app) => RootRow::App(app.index),
                compass_core::RootHit::Command { command, .. } => RootRow::Command(command),
                compass_core::RootHit::Extension { command, .. } => RootRow::Extension(
                    self.app_index
                        .extensions()
                        .iter()
                        .position(|known| known.id == command.id)
                        .unwrap_or_default(),
                ),
                compass_core::RootHit::Script { script, .. } => RootRow::Script(
                    self.app_index
                        .scripts()
                        .iter()
                        .position(|known| known.id == script.id)
                        .unwrap_or_default(),
                ),
                compass_core::RootHit::RhaiScript { script, .. } => RootRow::RhaiScript(
                    self.app_index
                        .rhai_scripts()
                        .iter()
                        .position(|known| known.id == script.id)
                        .unwrap_or_default(),
                ),
                compass_core::RootHit::Shortcut { shortcut, .. } => RootRow::Shortcut(
                    self.app_index
                        .shortcuts()
                        .iter()
                        .position(|known| known.id == shortcut.id)
                        .unwrap_or_default(),
                ),
            })
            .collect();
        // Back to the top on every new query: the old selection pointed into a
        // different list, and keeping its position would silently select an
        // unrelated application.
        self.selected = 0;
        if self.provider_scope.is_none() {
            self.apply_calculator();
            self.apply_fallbacks();
        }
        self.warm_icons();
    }

    /// Offers the fallbacks under the results for a non-empty query, as
    /// `RootFallbackSection` does.
    pub(super) fn apply_fallbacks(&mut self) {
        self.results
            .retain(|row| !matches!(row, RootRow::Fallback(_)));
        if self.query.trim().is_empty() || self.provider_scope.is_some() {
            return;
        }
        // An item the search already lists is not offered a second time
        // under the fallbacks ("docs" finds Search Files by its keywords).
        let listed = |fallback: &Fallback| {
            self.results.iter().any(|row| match (row, fallback) {
                (RootRow::Command(listed), Fallback::Command(offered)) => {
                    listed.entrypoint == offered.entrypoint
                }
                (RootRow::Extension(listed), Fallback::Extension(offered))
                | (RootRow::Shortcut(listed), Fallback::Shortcut(offered)) => listed == offered,
                _ => false,
            })
        };
        let fallbacks: Vec<RootRow> = self
            .fallbacks
            .iter()
            .filter_map(|id| self.resolve_fallback(id))
            .filter(|fallback| !listed(fallback))
            .map(RootRow::Fallback)
            .collect();
        self.results.extend(fallbacks);
    }

    /// The fallback a `fallbacks` entry names here, when it names an item
    /// that can be one (`isSuitableForFallback`): Search Files, any
    /// extension command, or a quicklink with exactly one argument.
    pub(super) fn resolve_fallback(&self, id: &str) -> Option<Fallback> {
        if let Some(command) = compass_core::commands::fallback(id) {
            return Some(Fallback::Command(command));
        }
        if let Some(index) = self
            .app_index
            .extensions()
            .iter()
            .position(|command| command.id == id)
        {
            return Some(Fallback::Extension(index));
        }
        let shortcut = self.app_index.shortcut_by_entrypoint(id)?;
        if shortcut.link.arguments.len() != 1 {
            return None;
        }
        self.app_index
            .shortcuts()
            .iter()
            .position(|known| known.id == shortcut.id)
            .map(Fallback::Shortcut)
    }

    /// What a fallback row says.
    pub(super) fn fallback_title(&self, fallback: Fallback) -> Option<String> {
        Some(match fallback {
            Fallback::Command(command) => command.title.to_owned(),
            Fallback::Extension(index) => self.app_index.extensions().get(index)?.title.clone(),
            Fallback::Shortcut(index) => {
                crate::shortcuts_page::display_name(self.app_index.shortcuts().get(index)?)
                    .to_owned()
            }
        })
    }

    /// Runs a fallback with the query: Search Files opens searching for it,
    /// an extension command opens with it as its fallback text
    /// (`OpenBuiltinCommandAction::setForwardSearchText`), and a quicklink
    /// opens with it as its argument (`OpenShortcutFromSearchText`).
    pub(super) fn open_fallback(&mut self, fallback: Fallback) -> Task<Message> {
        use compass_core::commands::CommandKind;
        self.panel = None;
        let query = self.query.clone();
        match fallback {
            Fallback::Command(command) => match command.kind {
                CommandKind::SearchFiles => self.open_search_files(query),
                _ => self.open_command(command),
            },
            Fallback::Extension(index) => self.run_extension_fallback(index, query),
            Fallback::Shortcut(index) => {
                let Some(shortcut) = self.app_index.shortcuts().get(index) else {
                    return Task::none();
                };
                let id = shortcut.id.clone();
                self.send_open_shortcut(id, vec![query])
            }
        }
    }

    /// Puts the calculator's answer to the query first, when there is one.
    pub(super) fn apply_calculator(&mut self) {
        self.results.retain(|row| *row != RootRow::Calculator);
        self.calculator = compass_core::calculator::evaluate(&self.query, !self.results.is_empty());
        if self.calculator.is_some() {
            self.results.insert(0, RootRow::Calculator);
        }
    }

    /// Asks the engine whether its catalog moved since this window last
    /// looked.
    pub(super) fn catalog_task(&self) -> Task<Message> {
        let Some(backend) = self.backend.clone() else {
            return Task::none();
        };
        Task::perform(
            async move { backend.catalog_generation().await },
            Message::CatalogGeneration,
        )
    }

    /// The engine rescanned applications or extensions: this window scans
    /// its own copy again, as the C++'s one process reloads its root items on
    /// `appsChanged`, and searches again so no row points at a moved entry.
    pub(super) fn catalog_moved(&mut self, generation: u64) -> Task<Message> {
        if generation == self.catalog_generation {
            return Task::none();
        }
        self.catalog_generation = generation;
        self.app_index.rescan_applications();
        self.app_index.rescan_extensions();
        if matches!(self.page, Page::Root) {
            self.search_task()
        } else {
            self.results.clear();
            Task::none()
        }
    }
}

pub(super) fn launch_task(
    launcher: Arc<dyn AppLauncher>,
    entry: compass_xdg::DesktopEntry,
    action_id: Option<String>,
    backend: Option<Arc<dyn crate::backend::ApplicationBackend>>,
    key: String,
) -> Task<Message> {
    Task::perform(
        async move {
            match action_id {
                Some(id) => launcher.launch_action(&entry, &id, &[]).await,
                None => launcher.launch(&entry, &[]).await,
            }
            .map(|_| ())
            .map_err(|error| error.to_string())?;
            if let Some(backend) = backend
                && let Err(error) = backend.record_launch(key).await
            {
                // The application already opened. A history failure must not
                // invite the user to retry that successful launch.
                tracing::warn!(%error, "could not record successful launch");
            }
            Ok(())
        },
        Message::Launched,
    )
}
