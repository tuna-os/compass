//! The settings view in the launcher: the C++ settings window's sidebar and
//! pages, each control writing its setting through the engine.
//!
//! A child module of `app` so it can reach the launcher's state; the model is
//! [`crate::settings_page`].

use iced::keyboard::{Key, Modifiers, key::Named};
use iced::widget::{button, pick_list, toggler};

use super::{
    Direction, Element, LauncherApp, Length, Message, Padding, Page, Task, chord_direction, column,
    container, focus_search, mouse_area, row, scrollable, text, text_input,
};
use crate::settings::SidebarKind;
use crate::settings_page::{
    DOCS_URL, HINT, ItemEntry, ProviderEntry, RecordTarget, SettingsMessage, SettingsPage, Shown,
    alias_key, providers_of,
};
use crate::shortcut_recorder::{Outcome as RecorderOutcome, ShortcutRecorder};
use compass_core::root_items::RootEdit;
use compass_core::settings_catalog::{self, CorePage, Kind, Setting};

/// The settings body's height: the card less its search field.
const BODY_HEIGHT: f32 = 430.0;

/// The sidebar's width.
const SIDEBAR_WIDTH: f32 = 190.0;

fn settings(message: SettingsMessage) -> Message {
    Message::Settings(message)
}

impl LauncherApp {
    /// Opens the settings at `tab` (`SettingsController::openTab`,
    /// `openExtensionPreferences`), reading the configuration file afresh.
    pub(super) fn open_settings(&mut self, tab: Option<&str>) -> Task<Message> {
        let (config, notice) = match &self.config_path {
            Some(path) => match compass_core::Config::load_from(path) {
                Ok(config) => (config, None),
                Err(err) => (
                    compass_core::Config::default(),
                    Some(format!("{err}; fix it before changing settings here")),
                ),
            },
            None => (compass_core::Config::default(), None),
        };
        let providers = providers_of(&self.app_index, &self.root_config);
        let themes = crate::theme::Theme::ALL
            .iter()
            .copied()
            .chain(crate::theme::load_user_themes(&self.theme_dirs))
            .map(|theme| (theme.name().to_owned(), theme.title().to_owned()))
            .collect();
        let mut page = SettingsPage::new(config, providers, themes, tab);
        page.notice = notice;
        self.panel = None;
        let _ = self.close_extension_view();
        self.page = Page::Settings(Box::new(page));
        focus_search()
    }

    /// Moves the settings page aside while an extension command's
    /// preferences form is open over it.
    pub(super) fn park_settings(&mut self) {
        if matches!(self.page, Page::Settings(_))
            && let Page::Settings(page) = std::mem::replace(&mut self.page, Page::Root)
        {
            self.parked_settings = Some(page);
        }
    }

    /// Back to the settings from a command's preferences form, when the form
    /// was opened from them.
    pub(super) fn back_to_settings(&mut self) -> Option<Task<Message>> {
        let Page::Preferences(form) = &self.page else {
            return None;
        };
        if form.purpose != crate::preferences_page::Purpose::CommandPreferences {
            return None;
        }
        let page = self.parked_settings.take()?;
        self.page = Page::Settings(page);
        Some(focus_search())
    }

    /// The view's keys: the arrows move through the sidebar, Tab through the
    /// fields, Escape leaves.
    pub(super) fn settings_page_key(&mut self, key: &Key, modifiers: Modifiers) -> Task<Message> {
        let Page::Settings(page) = &mut self.page else {
            return Task::none();
        };
        let direction = match key.as_ref() {
            Key::Named(Named::Escape) => {
                self.page = Page::Root;
                return focus_search();
            }
            Key::Named(Named::Tab) if modifiers.shift() => {
                return iced::widget::operation::focus_previous();
            }
            Key::Named(Named::Tab) => return iced::widget::operation::focus_next(),
            Key::Named(Named::ArrowDown) => Some(Direction::Down),
            Key::Named(Named::ArrowUp) => Some(Direction::Up),
            _ => chord_direction(self.keybinding, key.as_ref(), modifiers),
        };
        if let Some(direction) = direction {
            page.step(direction == Direction::Down);
            page.notice = None;
        }
        Task::none()
    }

    /// A key while a shortcut is recorded: the recorder takes it.
    pub(super) fn settings_recorder_event(
        &mut self,
        event: &iced::keyboard::Event,
    ) -> Task<Message> {
        let bound = self.recorder_bound();
        let launcher_hotkey = self.launcher_hotkey.clone();
        let Page::Settings(page) = &mut self.page else {
            return Task::none();
        };
        let Some((target, recorder)) = page.recorder.as_mut() else {
            return Task::none();
        };
        let outcome = recorder.key(
            event,
            Some(&launcher_hotkey),
            bound
                .iter()
                .map(|(id, title, shortcut)| (id.as_str(), title.as_str(), shortcut.as_str())),
        );
        let target = target.clone();
        self.settings_recorder_outcome(target, outcome)
    }

    /// What the settings view's recorder, recording for `target`, said to do.
    pub(super) fn settings_recorder_outcome(
        &mut self,
        target: RecordTarget,
        outcome: RecorderOutcome,
    ) -> Task<Message> {
        let Page::Settings(page) = &mut self.page else {
            return Task::none();
        };
        match outcome {
            RecorderOutcome::Recording => Task::none(),
            RecorderOutcome::Probe(trigger) => self.probe_shortcut(trigger),
            RecorderOutcome::Back => {
                page.recorder = None;
                focus_search()
            }
            RecorderOutcome::Save(shortcut) => {
                page.recorder = None;
                match target {
                    RecordTarget::Setting(key) => {
                        self.change_setting(key, serde_json::Value::String(shortcut))
                    }
                    RecordTarget::Item(id) => {
                        self.edit_settings_item(id, RootEdit::Shortcut(shortcut))
                    }
                }
            }
        }
    }

    /// Handles the view's messages.
    pub(super) fn settings_message(&mut self, message: SettingsMessage) -> Task<Message> {
        match message {
            SettingsMessage::QueryChanged(query) => {
                if let Page::Settings(page) = &mut self.page {
                    page.set_query(query);
                }
                Task::none()
            }
            SettingsMessage::SidebarSelected(row) => {
                if let Page::Settings(page) = &mut self.page {
                    page.select(row);
                    page.notice = None;
                }
                focus_search()
            }
            SettingsMessage::Changed(key, value) => self.change_setting(key, value),
            SettingsMessage::DraftEdited(key, draft) => {
                if let Page::Settings(page) = &mut self.page {
                    page.drafts.insert(key, draft);
                }
                Task::none()
            }
            SettingsMessage::DraftSubmitted(key) => {
                let Page::Settings(page) = &mut self.page else {
                    return Task::none();
                };
                let Some(draft) = page.drafts.get(&key).cloned() else {
                    return Task::none();
                };
                if let Some(id) = key.strip_prefix("alias:") {
                    return self.edit_settings_item(id.to_owned(), RootEdit::Alias(draft));
                }
                let Some(setting) = settings_catalog::find(&key) else {
                    return Task::none();
                };
                match SettingsPage::parse_draft(&setting, &draft) {
                    Ok(value) => self.change_setting(key, value),
                    Err(reason) => {
                        page.notice = Some(reason);
                        Task::none()
                    }
                }
            }
            SettingsMessage::Saved { key, result } => match result {
                Ok(()) => {
                    self.apply_setting_live(&key);
                    Task::none()
                }
                Err(reason) => {
                    if settings_catalog::find(&key).is_some_and(|s| s.kind == Kind::Theme) {
                        let _ = self.update(Message::ThemeCancel);
                    }
                    if let Page::Settings(page) = &mut self.page {
                        page.notice = Some(reason);
                    }
                    Task::none()
                }
            },
            SettingsMessage::Record(target) => {
                let Page::Settings(page) = &mut self.page else {
                    return Task::none();
                };
                let (id, title, current) = match &target {
                    RecordTarget::Setting(key) => {
                        let Some(setting) = settings_catalog::find(key) else {
                            return Task::none();
                        };
                        let current = page.value(&setting).as_str().map(str::to_owned);
                        (key.clone(), setting.label.to_owned(), current)
                    }
                    RecordTarget::Item(id) => {
                        let Some(item) = page
                            .providers
                            .iter()
                            .flat_map(|provider| &provider.items)
                            .find(|item| &item.id == id)
                        else {
                            return Task::none();
                        };
                        (id.clone(), item.title.clone(), item.shortcut.clone())
                    }
                };
                page.recorder = Some((target, ShortcutRecorder::new(id, title, current)));
                Task::none()
            }
            SettingsMessage::ProviderToggled(provider, enabled) => {
                if let Page::Settings(page) = &mut self.page {
                    page.set_provider_enabled(&provider, enabled);
                }
                compass_core::root_items::set_provider_enabled(
                    &mut self.root_config,
                    &provider,
                    enabled,
                );
                self.app_index.apply_root_config(&self.root_config);
                match self.backend.clone() {
                    Some(backend) => Task::perform(
                        async move { backend.set_provider_enabled(provider, enabled).await },
                        |result| settings(SettingsMessage::Done(result)),
                    ),
                    None => {
                        if let Err(reason) = self.write_settings_file()
                            && let Page::Settings(page) = &mut self.page
                        {
                            page.notice = Some(reason);
                        }
                        Task::none()
                    }
                }
            }
            SettingsMessage::ItemToggled(id, enabled) => {
                self.edit_settings_item(id, RootEdit::Enabled(enabled))
            }
            SettingsMessage::OpenPreferences(id) => {
                let Some(backend) = self.backend.clone() else {
                    if let Page::Settings(page) = &mut self.page {
                        page.notice = Some(crate::settings_page::NEEDS_ENGINE.to_owned());
                    }
                    return Task::none();
                };
                self.park_settings();
                let opened = id.clone();
                Task::perform(
                    async move { backend.extension_preferences(id).await },
                    move |result| Message::PreferencesOpened {
                        id: opened.clone(),
                        result,
                    },
                )
            }
            SettingsMessage::OpenUrl(url) => {
                let Some(backend) = self.backend.clone() else {
                    if let Page::Settings(page) = &mut self.page {
                        page.notice = Some(crate::settings_page::NEEDS_ENGINE.to_owned());
                    }
                    return Task::none();
                };
                let opened = Task::perform(async move { backend.open_url(url).await }, |result| {
                    settings(SettingsMessage::Done(result))
                });
                Task::batch([opened, self.conceal()])
            }
            SettingsMessage::Done(Ok(())) => Task::none(),
            SettingsMessage::Done(Err(reason)) => {
                match &mut self.page {
                    Page::Settings(page) => page.notice = Some(reason),
                    _ => self.error = Some(reason),
                }
                Task::none()
            }
        }
    }

    /// Writes a setting: into the view's copy at once, then through the
    /// engine (or, with none, straight into the file).
    fn change_setting(&mut self, key: String, value: serde_json::Value) -> Task<Message> {
        let Page::Settings(page) = &mut self.page else {
            return Task::none();
        };
        let Some(setting) = settings_catalog::find(&key) else {
            return Task::none();
        };
        if let Err(reason) = page.apply(&key, value.clone()) {
            page.notice = Some(reason);
            return Task::none();
        }
        // A theme shows at once, as Set Theme previews it; the engine's
        // answer keeps it or puts the old one back.
        let preview = match (&setting.kind, value.as_str()) {
            (Kind::Theme, Some(name)) => match crate::theme::Theme::from_name(name) {
                Some(theme) => self.update(Message::ThemePreview(theme)),
                None => Task::none(),
            },
            _ => Task::none(),
        };
        let saved = match self.backend.clone() {
            Some(backend) => {
                let answer = key.clone();
                Task::perform(
                    async move { backend.set_setting(key, value).await },
                    move |result| {
                        settings(SettingsMessage::Saved {
                            key: answer.clone(),
                            result,
                        })
                    },
                )
            }
            None => {
                let result = self.write_settings_file();
                self.settings_message(SettingsMessage::Saved { key, result })
            }
        };
        Task::batch([preview, saved])
    }

    /// Without an engine, the view's copy of the file is written where the
    /// launcher reads it.
    fn write_settings_file(&self) -> Result<(), String> {
        let (Page::Settings(page), Some(path)) = (&self.page, &self.config_path) else {
            return Err(crate::settings_page::NEEDS_ENGINE.to_owned());
        };
        page.config
            .save_to(path)
            .map_err(|err| format!("could not save the configuration: {err}"))
    }

    /// An item's switch, alias or shortcut: in the view, in this window's
    /// root search, and kept by the engine.
    fn edit_settings_item(&mut self, id: String, edit: RootEdit) -> Task<Message> {
        if let Page::Settings(page) = &mut self.page {
            page.edit_item(&id, &edit);
        }
        if self.backend.is_none()
            && let Err(reason) = self.write_settings_file()
            && let Page::Settings(page) = &mut self.page
        {
            page.notice = Some(reason);
        }
        self.edit_root_item(id, edit)
    }

    /// What this window holds of a setting it has just kept: the navigation
    /// keys, the layout, the clock, the font and the theme apply at once;
    /// the rest is the engine's.
    fn apply_setting_live(&mut self, key: &str) {
        let Page::Settings(page) = &self.page else {
            return;
        };
        let config = page.config.clone();
        let launcher = config.launcher();
        match key {
            "launcher.wrap_navigation" => self.wrap_navigation = launcher.wrap_navigation(),
            "launcher.keybinding" => self.keybinding = launcher.keybinding_scheme(),
            "launcher.quick_launch" => self.quick_launch = launcher.quick_launch(),
            "launcher.close_on_focus_loss" => {
                self.close_on_focus_loss = launcher.close_on_focus_loss();
            }
            "launcher.hotkey" => self.launcher_hotkey = launcher.hotkey().to_owned(),
            "launcher.clock.enabled" | "launcher.clock.format" | "launcher.clock.interval" => {
                let clock = launcher.clock();
                self.clock = clock.enabled().then(|| super::ClockSettings {
                    format: clock.format().to_owned(),
                    interval: clock.interval(),
                });
            }
            "launcher.appearance.preset"
            | "launcher.appearance.icons"
            | "launcher.appearance.tint" => {
                let appearance = launcher.appearance();
                let resolved = crate::preset::resolve(
                    Some(appearance.preset()),
                    appearance.icons_override(),
                    appearance.tint_override(),
                );
                self.geometry = resolved.geometry;
                self.field_rule = resolved.field_rule;
                self.tint = resolved.tint;
                self.subtitles = resolved.subtitles;
                self.icons = resolved.icons;
            }
            "launcher.appearance.color_scheme" => match launcher.appearance().color_scheme() {
                "light" => self.appearance = crate::design::Appearance::Light,
                "dark" => self.appearance = crate::design::Appearance::Dark,
                _ => {}
            },
            "launcher.appearance.theme" => {
                let _ = self.update(Message::ThemeCommit);
            }
            "font.normal.family" => self.font_family = config.font_family().map(str::to_owned),
            _ if key.starts_with("providers.power.entrypoints.") => {
                for command in compass_core::power_commands::COMMANDS {
                    let preferences = config.entrypoint_preferences(
                        compass_core::power_commands::EXTENSION_ID,
                        command.id,
                    );
                    self.power_asks.insert(
                        command.id.to_owned(),
                        compass_core::power_commands::should_confirm(command, preferences),
                    );
                }
            }
            _ if key.starts_with("providers.core.entrypoints.search-emojis.") => {
                let preferences = config.entrypoint_preferences("core", "search-emojis");
                self.emoji_skin_tone = preferences
                    .and_then(|p| p.get("skinTone")?.as_str())
                    .map(str::to_owned);
                self.emoji_default_action = preferences
                    .and_then(|p| p.get("defaultAction")?.as_str())
                    .unwrap_or(compass_core::emoji_grid::DEFAULT_ACTION_PASTE)
                    .to_owned();
            }
            _ => {}
        }
    }

    /// The view's body: the sidebar, the page, and a line for what went
    /// wrong and what the keys do.
    pub(super) fn settings_body<'a>(&'a self, page: &'a SettingsPage) -> Element<'a, Message> {
        let content: Element<'a, Message> = match (&page.recorder, page.shown()) {
            (Some((_, recorder)), _) => self.settings_recorder(recorder),
            (None, Shown::Core(core)) => self.settings_core_page(page, core),
            (None, Shown::Provider(provider)) => self.settings_provider_page(page, provider),
            (None, Shown::Nothing) => self.notice("No settings match"),
        };
        let palette = self.palette();
        let footer = text(page.notice.clone().unwrap_or_else(|| HINT.to_owned()))
            .font(self.font())
            .size(12)
            .color(if page.notice.is_some() {
                palette.accent.to_iced()
            } else {
                palette.muted.to_iced()
            });
        column![
            row![
                self.settings_sidebar(page),
                container(scrollable(container(content).padding(Padding::new(12.0))))
                    .width(Length::Fill)
                    .height(Length::Fill),
            ]
            .height(Length::Fixed(BODY_HEIGHT)),
            container(footer).padding(Padding::new(6.0).left(14)),
        ]
        .into()
    }

    fn settings_sidebar<'a>(&'a self, page: &'a SettingsPage) -> Element<'a, Message> {
        let palette = self.palette();
        let mut list = column![].spacing(2);
        for (position, entry) in page.sidebar.rows().iter().enumerate() {
            if entry.kind == SidebarKind::Divider {
                list = list
                    .push(container(iced::widget::rule::horizontal(1)).padding(Padding::new(4.0)));
                continue;
            }
            let selected = position as isize == page.selected;
            let colour = if selected {
                palette.selection_text
            } else if entry.enabled {
                palette.text
            } else {
                palette.muted
            }
            .to_iced();
            let background = selected.then(|| palette.selection.to_iced());
            let label = container(
                text(entry.label.clone())
                    .font(self.font())
                    .size(13)
                    .color(colour),
            )
            .width(Length::Fill)
            .padding(Padding::new(6.0).left(10))
            .style(move |_: &iced::Theme| container::Style {
                background: background.map(Into::into),
                border: iced::Border {
                    radius: 6.0.into(),
                    ..iced::Border::default()
                },
                ..container::Style::default()
            });
            list = list.push(
                mouse_area(label).on_press(settings(SettingsMessage::SidebarSelected(position))),
            );
        }
        container(scrollable(container(list).padding(Padding::new(8.0))))
            .width(Length::Fixed(SIDEBAR_WIDTH))
            .height(Length::Fill)
            .into()
    }

    fn settings_heading(&self, label: String) -> Element<'_, Message> {
        text(label)
            .font(iced::Font {
                weight: iced::font::Weight::Bold,
                ..self.font()
            })
            .size(16)
            .into()
    }

    /// One labelled row: the label and its description, the control on the
    /// right.
    fn settings_row<'a>(
        &'a self,
        label: String,
        description: String,
        control: Element<'a, Message>,
    ) -> Element<'a, Message> {
        let mut words = column![text(label).font(self.font()).size(13)].spacing(2);
        if !description.is_empty() {
            words = words.push(
                text(description)
                    .font(self.font())
                    .size(11)
                    .color(self.palette().muted.to_iced()),
            );
        }
        row![container(words).width(Length::Fill), control]
            .spacing(12)
            .align_y(iced::Alignment::Center)
            .into()
    }

    /// A setting's control, by its kind.
    fn settings_control<'a>(
        &'a self,
        page: &'a SettingsPage,
        setting: &Setting,
    ) -> Element<'a, Message> {
        let key = setting.key.clone();
        let value = page.value(setting);
        match &setting.kind {
            Kind::Toggle => {
                let on = value.as_bool().unwrap_or(false);
                toggler(on)
                    .on_toggle(move |on| {
                        settings(SettingsMessage::Changed(
                            key.clone(),
                            serde_json::Value::Bool(on),
                        ))
                    })
                    .into()
            }
            Kind::Choice(options) => {
                let labels: Vec<String> = options.iter().map(|(_, l)| (*l).to_owned()).collect();
                let current = value.as_str().and_then(|value| {
                    options
                        .iter()
                        .find(|(option, _)| *option == value)
                        .map(|(_, label)| (*label).to_owned())
                });
                let options = options.clone();
                pick_list(labels, current, move |label: String| {
                    let chosen = options
                        .iter()
                        .find(|(_, l)| *l == label)
                        .map_or("", |(option, _)| *option);
                    settings(SettingsMessage::Changed(
                        key.clone(),
                        serde_json::Value::String(chosen.to_owned()),
                    ))
                })
                .text_size(13)
                .into()
            }
            Kind::Theme => {
                let titles: Vec<String> = page.themes.iter().map(|(_, t)| t.clone()).collect();
                let current = value.as_str().and_then(|value| {
                    page.themes
                        .iter()
                        .find(|(name, _)| name.eq_ignore_ascii_case(value))
                        .map(|(_, title)| title.clone())
                });
                let themes = page.themes.clone();
                pick_list(titles, current, move |title: String| {
                    let name = themes
                        .iter()
                        .find(|(_, t)| *t == title)
                        .map_or_else(String::new, |(name, _)| name.clone());
                    settings(SettingsMessage::Changed(
                        key.clone(),
                        serde_json::Value::String(name),
                    ))
                })
                .text_size(13)
                .into()
            }
            Kind::Shortcut => {
                let current = value
                    .as_str()
                    .and_then(compass_core::key_combo::KeyCombo::parse)
                    .map_or_else(
                        || "Record Shortcut".to_owned(),
                        |combo| combo.display_tokens().join(" "),
                    );
                button(text(current).font(self.font()).size(13))
                    .on_press(settings(SettingsMessage::Record(RecordTarget::Setting(
                        key,
                    ))))
                    .into()
            }
            Kind::Number { .. } | Kind::Text | Kind::Paths | Kind::Names | Kind::Font => {
                let submit = key.clone();
                let placeholder = if setting.placeholder.is_empty() {
                    match setting.kind {
                        Kind::Paths => "Folders, separated by :",
                        Kind::Names => "Application ids, separated by ,",
                        _ => "",
                    }
                } else {
                    setting.placeholder
                };
                text_input(placeholder, &page.text_of(setting))
                    .font(self.font())
                    .size(13)
                    .padding(6)
                    .width(Length::Fixed(220.0))
                    .on_input(move |draft| {
                        settings(SettingsMessage::DraftEdited(key.clone(), draft))
                    })
                    .on_submit(settings(SettingsMessage::DraftSubmitted(submit)))
                    .into()
            }
        }
    }

    /// Settings grouped under their section headings.
    fn settings_list<'a>(
        &'a self,
        page: &'a SettingsPage,
        settings_shown: &[Setting],
    ) -> Element<'a, Message> {
        let mut list = column![].spacing(10);
        let mut section = "";
        for setting in settings_shown {
            if setting.section != section {
                section = setting.section;
                list = list.push(self.section_heading(section.to_owned()));
            }
            list = list.push(self.settings_row(
                setting.label.to_owned(),
                setting.description.to_owned(),
                self.settings_control(page, setting),
            ));
        }
        list.into()
    }

    fn settings_core_page<'a>(
        &'a self,
        page: &'a SettingsPage,
        core: CorePage,
    ) -> Element<'a, Message> {
        let mut body = column![self.settings_heading(core.title().to_owned())].spacing(12);
        match core {
            CorePage::About => {
                let muted = self.palette().muted.to_iced();
                body = body
                    .push(
                        text(format!("Compass {}", env!("CARGO_PKG_VERSION")))
                            .font(self.font())
                            .size(14),
                    )
                    .push(
                        text("A launcher for the Linux desktop, compatible with Vicinae.")
                            .font(self.font())
                            .size(12)
                            .color(muted),
                    );
                if let Some(offer) = &self.update {
                    body = body.push(
                        row![
                            text(super::release_check::title(offer))
                                .font(self.font())
                                .size(13),
                            button(text("View Release Notes").font(self.font()).size(13)).on_press(
                                settings(SettingsMessage::OpenUrl(offer.release_url.clone()))
                            ),
                        ]
                        .spacing(8)
                        .align_y(iced::Alignment::Center),
                    );
                }
                body = body.push(
                    row![
                        button(text("Documentation").font(self.font()).size(13))
                            .on_press(settings(SettingsMessage::OpenUrl(DOCS_URL.to_owned()))),
                        button(text("Report a Bug").font(self.font()).size(13)).on_press(settings(
                            SettingsMessage::OpenUrl(
                                compass_core::bug_report::CREATE_ISSUE_URL.to_owned()
                            )
                        )),
                    ]
                    .spacing(8),
                );
            }
            CorePage::Keybindings => {
                for (name, description, keys) in settings_catalog::KEYBINDINGS {
                    body = body.push(self.settings_row(
                        (*name).to_owned(),
                        (*description).to_owned(),
                        text(*keys).font(self.font()).size(13).into(),
                    ));
                }
            }
            CorePage::General | CorePage::Appearance | CorePage::Advanced => {
                body = body.push(self.settings_list(page, &SettingsPage::core_settings(core)));
            }
        }
        let missing = SettingsPage::not_in_compass(core);
        if !missing.is_empty() {
            let muted = self.palette().muted.to_iced();
            let mut notes = column![self.section_heading("Not in Compass".to_owned())].spacing(4);
            for item in missing {
                notes = notes.push(
                    text(format!("{}: {}", item.label, item.reason))
                        .font(self.font())
                        .size(11)
                        .color(muted),
                );
            }
            body = body.push(notes);
        }
        body.into()
    }

    fn settings_provider_page<'a>(
        &'a self,
        page: &'a SettingsPage,
        provider: &'a ProviderEntry,
    ) -> Element<'a, Message> {
        let id = provider.id.clone();
        let header = self.settings_row(
            provider.title.clone(),
            format!("{} · {} items", provider.provenance, provider.items.len()),
            toggler(provider.enabled)
                .on_toggle(move |on| settings(SettingsMessage::ProviderToggled(id.clone(), on)))
                .into(),
        );
        let mut body = column![header].spacing(12);
        let own = SettingsPage::provider_settings(&provider.id);
        if !own.is_empty() {
            body = body.push(self.settings_list(page, &own));
        }
        for item in &provider.items {
            body = body.push(self.settings_item(page, item));
        }
        body.into()
    }

    /// One root item: its switch, alias, shortcut, preferences button, and
    /// the settings that belong to it.
    fn settings_item<'a>(
        &'a self,
        page: &'a SettingsPage,
        item: &'a ItemEntry,
    ) -> Element<'a, Message> {
        let id = item.id.clone();
        let toggle_id = id.clone();
        let alias = alias_key(&id);
        let submit = alias.clone();
        let alias_text = page
            .drafts
            .get(&alias)
            .cloned()
            .unwrap_or_else(|| item.alias.clone().unwrap_or_default());
        let shortcut = item
            .shortcut
            .as_deref()
            .and_then(compass_core::key_combo::KeyCombo::parse)
            .map_or_else(
                || "Record Shortcut".to_owned(),
                |combo| combo.display_tokens().join(" "),
            );
        let mut controls = row![
            text_input("Alias", &alias_text)
                .font(self.font())
                .size(12)
                .padding(4)
                .width(Length::Fixed(90.0))
                .on_input(move |draft| settings(SettingsMessage::DraftEdited(alias.clone(), draft)))
                .on_submit(settings(SettingsMessage::DraftSubmitted(submit))),
            button(text(shortcut).font(self.font()).size(12)).on_press(settings(
                SettingsMessage::Record(RecordTarget::Item(id.clone()))
            )),
        ]
        .spacing(6)
        .align_y(iced::Alignment::Center);
        if item.has_preferences {
            controls = controls.push(
                button(text("Preferences").font(self.font()).size(12))
                    .on_press(settings(SettingsMessage::OpenPreferences(id.clone()))),
            );
        }
        controls = controls
            .push(toggler(item.enabled).on_toggle(move |on| {
                settings(SettingsMessage::ItemToggled(toggle_id.clone(), on))
            }));
        let mut entry =
            column![self.settings_row(item.title.clone(), String::new(), controls.into())]
                .spacing(8);
        let own = SettingsPage::item_settings(&item.id);
        if !own.is_empty() {
            entry = entry.push(
                container(self.settings_list(page, &own)).padding(Padding::new(0.0).left(16)),
            );
        }
        entry.into()
    }

    fn settings_recorder<'a>(&'a self, recorder: &'a ShortcutRecorder) -> Element<'a, Message> {
        let palette = self.palette();
        let badge = if recorder.tokens.is_empty() {
            "…".to_owned()
        } else {
            recorder.tokens.join(" ")
        };
        let mut body = column![
            self.settings_heading(recorder.title.clone()),
            text(badge).font(self.font()).size(20),
            text(recorder.status.clone())
                .font(self.font())
                .size(12)
                .color(if recorder.error {
                    palette.accent.to_iced()
                } else {
                    palette.muted.to_iced()
                }),
        ]
        .spacing(12)
        .align_x(iced::Alignment::Center);
        if recorder.current.is_some() {
            body = body.push(
                text(crate::shortcut_recorder::REMOVE_HINT)
                    .font(self.font())
                    .size(11)
                    .color(palette.muted.to_iced()),
            );
        }
        container(body).width(Length::Fill).padding(24).into()
    }
}

#[cfg(test)]
mod tests;
