//! The Vicinae extension's own commands (`VicinaeExtension`): Configure
//! Fallback Commands, and the commands that open a link or a file or refresh
//! something and hide.
//!
//! A child module of `app` so it can reach the launcher's state; the fallback
//! manager's model is [`crate::fallbacks_page`].

use iced::keyboard::{Key, Modifiers, key::Named};

use super::{
    Direction, Element, LauncherApp, Length, Message, Padding, Page, PanelSection, PanelState,
    Task, chord_direction, column, container, focus_search, mouse_area, next_selection, scrollable,
};
use crate::action_panel::Action;
use crate::fallbacks_page::{Candidate, FallbacksPage};
use crate::vicinae_pages::{ExtensionsPage, IconsPage, provenance_badge};
use compass_core::root_items::RootEdit;

/// The fallback manager's one action.
const FALLBACK_TOGGLE: &str = "fallback.toggle";
/// A fallback row's "Manage Fallback Actions" (`ManageFallbackActions`).
pub(super) const MANAGE_FALLBACKS: &str = "root.manage-fallbacks";
/// A fallback row's Open.
pub(super) const OPEN_FALLBACK: &str = "root.open-fallback";

/// Show Installed Extensions' actions.
const EXTENSION_UNINSTALL: &str = "extension.uninstall";
const EXTENSION_COPY_NAME: &str = "extension.copy-name";
const EXTENSION_COPY_ID: &str = "extension.copy-id";
const EXTENSION_COPY_PATH: &str = "extension.copy-path";
const EXTENSION_COPY_AUTHOR: &str = "extension.copy-author";
/// Search Builtin Icons' action.
const ICON_COPY_NAME: &str = "icon.copy-name";

/// What Reload Script Directories says (`ReloadScriptDirectoriesCommand`).
pub const SCRIPTS_RESCANNED: &str = "New scan triggered, index will update shortly";
/// What Refresh Apps says (`RefreshAppsCommand`).
pub const APPS_REFRESHED: &str = "Apps successfully refreshed";
/// What a link command's HUD says (`BuiltinUrlCommand`).
pub const OPENED_IN_BROWSER: &str = "Opened in browser";

impl LauncherApp {
    /// Runs one of the Vicinae extension's commands, by its C++ id.
    pub(super) fn open_vicinae_command(&mut self, id: &'static str) -> Task<Message> {
        match id {
            "manage-fallback" => self.open_manage_fallbacks(),
            "report-bug" => {
                let url = compass_core::bug_report::report_url(None, &system_info());
                self.open_link(url)
            }
            "sponsor" => self.open_link(compass_core::commands::SPONSOR_URL.to_owned()),
            "join-discord-server" => self.open_link(compass_core::commands::DISCORD_URL.to_owned()),
            "open-config-file" => match self.config_path.clone() {
                Some(path) => self.open_path_and_hide(path, false),
                None => self.say_in_root("No configuration file to open".to_owned()),
            },
            "open-default-config" => match write_default_config() {
                Ok(path) => self.open_path_and_hide(path, false),
                Err(reason) => self.say_in_root(reason),
            },
            "show-logs" => match compass_core::xdg_dirs::state_dir() {
                Some(dir) => self.open_path_and_hide(dir.join(LOG_FILE_NAME), true),
                None => self.say_in_root("No log file to show".to_owned()),
            },
            "reload-scripts" => {
                let rescan = self.refresh_scripts_task();
                Task::batch([rescan, self.say_in_root(SCRIPTS_RESCANNED.to_owned())])
            }
            "refresh-apps" => {
                self.app_index.rescan_applications();
                let search = self.search_task();
                Task::batch([search, self.say_in_root(APPS_REFRESHED.to_owned())])
            }
            _ => self.open_vicinae_view(id),
        }
    }

    /// The root list, its query cleared, with `line` where it says things,
    /// as a C++ callback command's toast over the root search.
    fn say_in_root(&mut self, line: String) -> Task<Message> {
        self.page = Page::Root;
        self.query.clear();
        self.error = Some(line);
        focus_search()
    }

    /// Opens `url` in the browser and hides with "Opened in browser", as
    /// `BuiltinUrlCommand`.
    fn open_link(&mut self, url: String) -> Task<Message> {
        let Some(backend) = self.backend.clone() else {
            return self.say_in_root("Opening a link needs the Compass engine".to_owned());
        };
        Task::perform(async move { backend.open_url(url).await }, |result| {
            Message::ActionDone(Some(crate::hud::Hud::new(OPENED_IN_BROWSER)), result)
        })
    }

    /// Opens `path` (or shows it in the file browser when `reveal`), then
    /// hides with the search text cleared, as `OpenVicinaeConfig`.
    fn open_path_and_hide(&mut self, path: std::path::PathBuf, reveal: bool) -> Task<Message> {
        let Some(backend) = self.backend.clone() else {
            return self.say_in_root("Opening a file needs the Compass engine".to_owned());
        };
        self.query.clear();
        let path = path.to_string_lossy().into_owned();
        Task::perform(
            async move { backend.open_file(path, reveal).await },
            |result| Message::ActionDone(None, result),
        )
    }

    /// Every item that can be a fallback, as `isSuitableForFallback`
    /// answers: Search Files, each extension command, and each quicklink
    /// with exactly one argument; each marked with the `fallbacks` entry
    /// that enables it.
    pub(super) fn fallback_candidates(&self) -> Vec<Candidate> {
        use compass_core::commands::{SEARCH_FILES_FALLBACK_ID, fallback};
        let enabled_as =
            |matches: &dyn Fn(&str) -> bool| self.fallbacks.iter().find(|id| matches(id)).cloned();
        let mut candidates = Vec::new();
        if let Some(files) = fallback(SEARCH_FILES_FALLBACK_ID) {
            candidates.push(Candidate {
                id: SEARCH_FILES_FALLBACK_ID.to_owned(),
                title: files.title.to_owned(),
                subtitle: files.subtitle.to_owned(),
                keywords: files.keywords.iter().map(|k| (*k).to_owned()).collect(),
                icon: Some(files.icon),
                enabled_as: enabled_as(&|id| fallback(id).is_some()),
            });
        }
        for command in self.app_index.extensions() {
            candidates.push(Candidate {
                id: command.id.clone(),
                title: command.title.clone(),
                subtitle: command.extension_title.clone(),
                keywords: command.keywords.clone(),
                icon: None,
                enabled_as: enabled_as(&|id| id == command.id),
            });
        }
        for shortcut in self.app_index.shortcuts() {
            if shortcut.link.arguments.len() != 1 {
                continue;
            }
            let id = compass_core::root_items::entrypoint_id(
                compass_core::shortcut::SHORTCUTS_PROVIDER_ID,
                &shortcut.id,
            );
            candidates.push(Candidate {
                title: crate::shortcuts_page::display_name(shortcut).to_owned(),
                subtitle: "Shortcut".to_owned(),
                keywords: Vec::new(),
                icon: None,
                enabled_as: enabled_as(&|known| known == id),
                id,
            });
        }
        candidates
    }

    /// Opens Configure Fallback Commands.
    pub(super) fn open_manage_fallbacks(&mut self) -> Task<Message> {
        self.panel = None;
        self.page = Page::Fallbacks(FallbacksPage::new(
            self.fallback_candidates(),
            self.fallbacks.clone(),
        ));
        focus_search()
    }

    /// Enables or disables the selected item, in this window at once and in
    /// the configuration through the engine, then lists again.
    fn toggle_selected_fallback(&mut self) -> Task<Message> {
        let Page::Fallbacks(page) = &self.page else {
            return Task::none();
        };
        let Some((_, id, enable)) = page.selected_action() else {
            return Task::none();
        };
        self.panel = None;
        compass_core::root_items::set_fallback(&mut self.fallbacks, &id, enable);
        let candidates = self.fallback_candidates();
        let order = self.fallbacks.clone();
        if let Page::Fallbacks(page) = &mut self.page {
            page.notice = None;
            page.reload(candidates, order);
        }
        match self.backend.clone() {
            Some(backend) => Task::batch([
                Task::perform(
                    async move { backend.edit_root_item(id, RootEdit::Fallback(enable)).await },
                    Message::RootItemEdited,
                ),
                focus_search(),
            ]),
            None => focus_search(),
        }
    }

    /// A fallback row's panel in root search (`fallbackActionPanel`): Open
    /// and Manage Fallback Actions.
    pub(super) fn open_fallback_row_panel(&mut self) -> Option<Task<Message>> {
        if !matches!(self.page, Page::Root) {
            return None;
        }
        let super::RootRow::Fallback(fallback) = self.selected_row()? else {
            return None;
        };
        let open = match fallback {
            super::Fallback::Shortcut(_) => "Open",
            _ => "Open command",
        };
        self.panel = Some(PanelState::new(vec![PanelSection {
            name: String::new(),
            actions: vec![
                Action::new(open)
                    .with_id(OPEN_FALLBACK)
                    .with_shortcut("enter"),
                Action::new("Manage Fallback Actions")
                    .with_id(MANAGE_FALLBACKS)
                    .with_shortcut("ctrl+enter"),
            ],
        }]));
        Some(iced::widget::operation::focus(super::PANEL_INPUT))
    }

    /// Runs one of this module's panel actions, if `id` is one.
    pub(super) fn vicinae_panel_action(&mut self, id: &str) -> Option<Task<Message>> {
        match id {
            OPEN_FALLBACK => {
                self.panel = None;
                Some(self.update(Message::LaunchSelected))
            }
            MANAGE_FALLBACKS => Some(self.open_manage_fallbacks()),
            ICON_COPY_NAME => {
                let Page::Icons(page) = &self.page else {
                    return None;
                };
                let name = page.selected_name()?.to_owned();
                self.panel = None;
                Some(self.copy_with_hud(name))
            }
            EXTENSION_UNINSTALL
            | EXTENSION_COPY_NAME
            | EXTENSION_COPY_ID
            | EXTENSION_COPY_PATH
            | EXTENSION_COPY_AUTHOR => {
                let Page::Extensions(page) = &self.page else {
                    return None;
                };
                let extension = page.selected_extension()?;
                let text = match id {
                    EXTENSION_UNINSTALL => {
                        let id = extension.id.clone();
                        self.panel = None;
                        return Some(self.ask_to_uninstall(id));
                    }
                    EXTENSION_COPY_NAME => extension.name.clone(),
                    EXTENSION_COPY_ID => extension.id.clone(),
                    EXTENSION_COPY_PATH => extension.path.to_string_lossy().into_owned(),
                    _ => extension.author.clone(),
                };
                self.panel = None;
                Some(self.copy_with_hud(text))
            }
            _ => self.fallbacks_panel_action(id),
        }
    }

    /// Opens one of the inspection views, by its C++ id.
    fn open_vicinae_view(&mut self, id: &'static str) -> Task<Message> {
        self.panel = None;
        match id {
            "list-extensions" => {
                self.page = Page::Extensions(ExtensionsPage::new(self.installed_extensions()));
            }
            "search-builtin-icons" => self.page = Page::Icons(IconsPage::new()),
            _ => return Task::none(),
        }
        focus_search()
    }

    /// Every installed extension's manifest, read from the directories root
    /// search found commands in (`ExtensionRegistry::scanAll`).
    fn installed_extensions(&self) -> Vec<compass_core::manifest::ExtensionManifest> {
        let mut directories: Vec<&std::path::Path> = self
            .app_index
            .extensions()
            .iter()
            .map(|command| command.extension_dir.as_path())
            .collect();
        directories.sort_unstable();
        directories.dedup();
        directories
            .into_iter()
            .filter_map(|dir| compass_core::manifest::ExtensionManifest::from_directory(dir).ok())
            .collect()
    }

    /// `UninstallExtensionAction`: asks, then uninstalls through the engine.
    fn ask_to_uninstall(&mut self, id: String) -> Task<Message> {
        self.confirm = Some(super::Confirm {
            title: crate::store_page::CONFIRM_TITLE.to_owned(),
            message: crate::store_page::CONFIRM_MESSAGE.to_owned(),
            confirm_text: "Uninstall".to_owned(),
            action: super::ConfirmAction::UninstallExtension(id),
        });
        Task::none()
    }

    /// Uninstalls extension `id` through the engine.
    pub(super) fn uninstall_extension(&mut self, id: String) -> Task<Message> {
        let Some(backend) = self.backend.clone() else {
            if let Page::Extensions(page) = &mut self.page {
                page.notice = Some("Uninstalling needs the Compass engine".to_owned());
            }
            return Task::none();
        };
        Task::perform(
            {
                let id = id.clone();
                async move { backend.store_uninstall(id).await }
            },
            move |result| Message::ExtensionUninstalled {
                id: id.clone(),
                result,
            },
        )
    }

    /// The panel over the selected row of an inspection view.
    pub(super) fn open_vicinae_view_panel(&mut self) -> Option<Task<Message>> {
        let sections = match &self.page {
            Page::Icons(page) => {
                page.selected_name()?;
                vec![PanelSection {
                    name: String::new(),
                    actions: vec![
                        Action::new("Copy Icon Name")
                            .with_id(ICON_COPY_NAME)
                            .with_shortcut("enter"),
                    ],
                }]
            }
            Page::Extensions(page) => {
                let extension = page.selected_extension()?;
                let mut copies = vec![
                    Action::new("Copy Name").with_id(EXTENSION_COPY_NAME),
                    Action::new("Copy ID").with_id(EXTENSION_COPY_ID),
                    Action::new("Copy Path").with_id(EXTENSION_COPY_PATH),
                ];
                if !extension.author.is_empty() {
                    copies.push(Action::new("Copy Author").with_id(EXTENSION_COPY_AUTHOR));
                }
                vec![
                    PanelSection {
                        name: String::new(),
                        actions: vec![
                            Action::new("Uninstall")
                                .with_id(EXTENSION_UNINSTALL)
                                .with_shortcut("enter"),
                        ],
                    },
                    PanelSection {
                        name: "Copy".to_owned(),
                        actions: copies,
                    },
                ]
            }
            _ => return None,
        };
        self.panel = Some(PanelState::new(sections));
        Some(iced::widget::operation::focus(super::PANEL_INPUT))
    }

    /// The inspection views' keys: the arrows, Escape back, Enter for the
    /// first action.
    pub(super) fn vicinae_view_key(&mut self, key: &Key, modifiers: Modifiers) -> Task<Message> {
        let (len, selected) = match &mut self.page {
            Page::Icons(page) => (page.shown.len(), &mut page.selected),
            Page::Extensions(page) => (page.shown.len(), &mut page.selected),
            _ => return Task::none(),
        };
        let direction = match key.as_ref() {
            Key::Named(Named::ArrowDown) => Some(Direction::Down),
            Key::Named(Named::ArrowUp) => Some(Direction::Up),
            Key::Named(Named::Escape) => return self.update(Message::Back),
            Key::Named(Named::Enter) => return self.vicinae_view_primary(),
            _ => chord_direction(self.keybinding, key.as_ref(), modifiers),
        };
        if let Some(direction) = direction {
            *selected = next_selection(len, *selected, direction, self.wrap_navigation);
            return crate::scroll::reveal_root_selection();
        }
        Task::none()
    }

    /// Enter on an inspection view: its panel's first action.
    fn vicinae_view_primary(&mut self) -> Task<Message> {
        let id = match &self.page {
            Page::Icons(_) => ICON_COPY_NAME,
            Page::Extensions(_) => EXTENSION_UNINSTALL,
            _ => return Task::none(),
        };
        self.vicinae_panel_action(id).unwrap_or_else(Task::none)
    }

    /// Handles the inspection views' messages.
    pub(super) fn vicinae_view_message(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::ExtensionsQueryChanged(query) => {
                if let Page::Extensions(page) = &mut self.page {
                    page.query = query;
                    page.refilter();
                }
                crate::scroll::reveal_root_selection()
            }
            Message::IconsQueryChanged(query) => {
                if let Page::Icons(page) = &mut self.page {
                    page.query = query;
                    page.refilter();
                }
                crate::scroll::reveal_root_selection()
            }
            Message::VicinaeRowSelected(position) => {
                match &mut self.page {
                    Page::Icons(page) if position < page.shown.len() => page.selected = position,
                    Page::Extensions(page) if position < page.shown.len() => {
                        page.selected = position;
                    }
                    _ => return Task::none(),
                }
                self.vicinae_view_primary()
            }
            Message::ExtensionUninstalled { id, result } => {
                if let Page::Extensions(page) = &mut self.page {
                    match result {
                        Ok(()) => {
                            page.remove(&id);
                            page.notice = Some(crate::store_page::UNINSTALLED.to_owned());
                        }
                        Err(reason) => {
                            page.notice = Some(format!("Failed to uninstall extension: {reason}"));
                        }
                    }
                }
                self.catalog_task()
            }
            _ => Task::none(),
        }
    }

    /// Show Installed Extensions' and Search Builtin Icons' bodies.
    pub(super) fn vicinae_view_body(&self) -> Option<Element<'_, Message>> {
        let mut list = column![].spacing(f32::from(self.geometry.row_spacing));
        let notice = match &self.page {
            Page::Icons(page) => {
                if page.shown.is_empty() {
                    return Some(self.notice("No icons match"));
                }
                let names = compass_core::builtin_icon::names();
                for (position, &index) in page.shown.iter().enumerate() {
                    let Some(name) = names.get(index) else {
                        continue;
                    };
                    let selected = position == page.selected;
                    let glyph = crate::icons::Glyph::builtin(*name);
                    let icon = self.glyph_or_initial(Some(&glyph), name, selected);
                    list = list.push(self.vicinae_row(
                        self.list_row(icon, (*name).to_owned(), None, selected),
                        position,
                        selected,
                    ));
                }
                None
            }
            Page::Extensions(page) => {
                if page.all.is_empty() {
                    return Some(self.notice("No extensions are installed"));
                }
                if page.shown.is_empty() {
                    return Some(self.notice("No extensions match"));
                }
                for (position, &index) in page.shown.iter().enumerate() {
                    let Some(extension) = page.all.get(index) else {
                        continue;
                    };
                    let selected = position == page.selected;
                    let icon = self.initial_badge(&extension.title, selected);
                    let row = self.list_row_with(
                        icon,
                        extension.title.clone(),
                        self.subtitles.then(|| extension.description.clone()),
                        Some(provenance_badge(extension.provenance).to_owned()),
                        selected,
                    );
                    list = list.push(self.vicinae_row(row, position, selected));
                }
                page.notice.as_deref()
            }
            _ => return None,
        };
        let rows = scrollable(container(list).padding(Padding::new(6.0).top(8)))
            .id(crate::scroll::ROOT_RESULTS)
            .height(Length::Shrink);
        Some(match notice {
            Some(notice) => column![rows, self.notice(notice)].into(),
            None => rows.into(),
        })
    }

    /// A row of an inspection view: clickable, and marked for scrolling
    /// when selected.
    fn vicinae_row<'a>(
        &'a self,
        row: Element<'a, Message>,
        position: usize,
        selected: bool,
    ) -> Element<'a, Message> {
        let row: Element<Message> = mouse_area(row)
            .on_press(Message::VicinaeRowSelected(position))
            .into();
        if selected {
            container(row).id(crate::scroll::ROOT_SELECTION).into()
        } else {
            row
        }
    }

    /// The panel over the selected item: its one action.
    pub(super) fn open_fallbacks_panel(&mut self) -> Option<Task<Message>> {
        let Page::Fallbacks(page) = &self.page else {
            return None;
        };
        let (label, _, _) = page.selected_action()?;
        self.panel = Some(PanelState::new(vec![PanelSection {
            name: String::new(),
            actions: vec![
                Action::new(label)
                    .with_id(FALLBACK_TOGGLE)
                    .with_shortcut("enter"),
            ],
        }]));
        Some(iced::widget::operation::focus(super::PANEL_INPUT))
    }

    /// Runs a fallback manager panel action, if `id` is one.
    fn fallbacks_panel_action(&mut self, id: &str) -> Option<Task<Message>> {
        (id == FALLBACK_TOGGLE && matches!(self.page, Page::Fallbacks(_)))
            .then(|| self.toggle_selected_fallback())
    }

    /// The view's keys.
    pub(super) fn fallbacks_page_key(&mut self, key: &Key, modifiers: Modifiers) -> Task<Message> {
        let Page::Fallbacks(page) = &mut self.page else {
            return Task::none();
        };
        let direction = match key.as_ref() {
            Key::Named(Named::ArrowDown) => Some(Direction::Down),
            Key::Named(Named::ArrowUp) => Some(Direction::Up),
            Key::Named(Named::Escape) => return self.update(Message::Back),
            Key::Named(Named::Enter) => return self.toggle_selected_fallback(),
            _ => chord_direction(self.keybinding, key.as_ref(), modifiers),
        };
        if let Some(direction) = direction {
            page.selected = next_selection(
                page.rows.len(),
                page.selected,
                direction,
                self.wrap_navigation,
            );
            return crate::scroll::reveal_root_selection();
        }
        Task::none()
    }

    /// Handles the fallback manager's messages.
    pub(super) fn fallbacks_message(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::FallbacksQueryChanged(query) => {
                if let Page::Fallbacks(page) = &mut self.page {
                    page.query = query;
                    page.refilter();
                }
                crate::scroll::reveal_root_selection()
            }
            Message::FallbackSelected(position) => {
                if let Page::Fallbacks(page) = &mut self.page
                    && position < page.rows.len()
                {
                    page.selected = position;
                }
                self.toggle_selected_fallback()
            }
            _ => Task::none(),
        }
    }

    /// The fallback manager's body: "Enabled", then "Available".
    pub(super) fn fallbacks_body<'a>(&'a self, page: &'a FallbacksPage) -> Element<'a, Message> {
        if page.rows.is_empty() {
            return self.notice(if page.candidates.is_empty() {
                "Nothing can be a fallback"
            } else {
                "No commands match"
            });
        }
        let mut list = column![].spacing(f32::from(self.geometry.row_spacing));
        for (position, row) in page.rows.iter().enumerate() {
            let Some(candidate) = page.candidates.get(row.candidate) else {
                continue;
            };
            if let Some(heading) = page.heading_at(position) {
                list = list.push(self.section_heading(heading.to_owned()));
            }
            let selected = position == page.selected;
            let icon = match compass_core::commands::fallback(&candidate.id) {
                Some(command) => self.command_icon(command, selected),
                None => self.initial_badge(&candidate.title, selected),
            };
            let line = self.list_row(
                icon,
                candidate.title.clone(),
                self.subtitles.then(|| candidate.subtitle.clone()),
                selected,
            );
            let line: Element<Message> = mouse_area(line)
                .on_press(Message::FallbackSelected(position))
                .into();
            let line: Element<Message> = if selected {
                container(line).id(crate::scroll::ROOT_SELECTION).into()
            } else {
                line
            };
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
}

/// The log file under the state directory (`vicinae::logs::FILE_NAME`).
const LOG_FILE_NAME: &str = "compass.log";

/// `OpenDefaultVicinaeConfig`: the configuration at its defaults, written
/// read-only to the runtime directory as `default-config.jsonc` (replacing
/// the last one), for the editor to open.
fn write_default_config() -> Result<std::path::PathBuf, String> {
    let dir = std::env::var_os("XDG_RUNTIME_DIR")
        .filter(|dir| !dir.is_empty())
        .map_or_else(std::env::temp_dir, std::path::PathBuf::from)
        .join("vicinae");
    write_default_config_in(&dir)
}

fn write_default_config_in(dir: &std::path::Path) -> Result<std::path::PathBuf, String> {
    let failed = |_| "Failed to open temporary file".to_owned();
    std::fs::create_dir_all(dir).map_err(failed)?;
    let path = dir.join("default-config.jsonc");
    if path.exists() {
        std::fs::remove_file(&path).map_err(failed)?;
    }
    let text = serde_json::to_string_pretty(&compass_core::config::default_document())
        .map_err(|_| "Failed to open default config file".to_owned())?;
    std::fs::write(&path, text).map_err(failed)?;
    let mut permissions = std::fs::metadata(&path).map_err(failed)?.permissions();
    permissions.set_readonly(true);
    std::fs::set_permissions(&path, permissions).map_err(failed)?;
    Ok(path)
}

/// What the bug report is pre-filled with: this build and this machine.
fn system_info() -> compass_core::bug_report::SystemInfo {
    use compass_core::bug_report::{SystemInfo, os_description, parse_os_release};
    let profile = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    let os_release = std::fs::read_to_string("/etc/os-release")
        .ok()
        .and_then(|text| parse_os_release(&text));
    SystemInfo {
        version: env!("CARGO_PKG_VERSION").to_owned(),
        commit: option_env!("COMPASS_GIT_COMMIT")
            .unwrap_or("unknown")
            .to_owned(),
        build_info: format!(
            "rustc - {profile} - {}/{}",
            std::env::consts::OS,
            std::env::consts::ARCH
        ),
        provenance: option_env!("COMPASS_PROVENANCE")
            .unwrap_or("local")
            .to_owned(),
        os: os_description(
            os_release
                .as_ref()
                .map(|(name, version)| (name.as_str(), version.as_str())),
            std::env::consts::OS,
            std::env::consts::ARCH,
        ),
        qt_platform: "wayland".to_owned(),
        desktop: compass_core::xdg_dirs::current_desktops().join(":"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_config_is_written_read_only_and_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_default_config_in(dir.path()).unwrap();
        assert!(std::fs::metadata(&path).unwrap().permissions().readonly());
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(serde_json::from_str::<serde_json::Value>(&text).is_ok());
        assert_eq!(write_default_config_in(dir.path()).unwrap(), path, "again");
    }

    #[test]
    fn the_report_names_this_build() {
        let info = system_info();
        assert_eq!(info.version, env!("CARGO_PKG_VERSION"));
        assert!(info.build_info.starts_with("rustc - "));
    }
}
