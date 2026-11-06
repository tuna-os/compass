//! The launcher's forms: one `PreferencesPage` drawn for every purpose it
//! serves -- an extension command's preferences or arguments, a shortcut's,
//! a snippet's or a script's arguments, the shortcut and snippet editors,
//! Create Extension, an alias -- and saving or submitting what was entered.
//!
//! The purpose decides what Enter does with the values; see
//! `crate::preferences_page::Purpose`.

use super::{
    Border, Color, Element, LauncherApp, Length, Message, Padding, Page, Task, Theme, column,
    container, focus_search, scrollable, text_input,
};

impl LauncherApp {
    /// Editing and submitting the form on screen.
    pub(super) fn preferences_message(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::PreferenceEdited(index, value) => {
                if let Page::Preferences(page) = &mut self.page
                    && let Some(slot) = page.values.get_mut(index)
                {
                    *slot = value;
                    page.notice = None;
                    page.focus = Some(index);
                }
                Task::none()
            }
            Message::PreferenceTextEdited(index, action) => {
                if let Page::Preferences(page) = &mut self.page {
                    if action.is_edit() {
                        page.focus = Some(index);
                    }
                    page.edit_text_area(index, action);
                }
                Task::none()
            }
            Message::PreferencesSubmit => {
                if let Some(task) = self.submit_emoji_keywords() {
                    return task;
                }
                if let Some(task) = self.submit_clipboard_keywords() {
                    return task;
                }
                if let Some(task) = self.submit_alias_form() {
                    return task;
                }
                if let Some(task) = self.submit_shortcut_form() {
                    return task;
                }
                if let Some(task) = self.submit_snippet_form() {
                    return task;
                }
                if let Some(task) = self.submit_script_form() {
                    return task;
                }
                if let Some(task) = self.submit_create_extension() {
                    return task;
                }
                if let Some(task) = self.submit_media_form() {
                    return task;
                }
                let Page::Preferences(page) = &mut self.page else {
                    return Task::none();
                };
                let values = match page.submission() {
                    Ok(values) => values,
                    Err(missing) => {
                        page.notice = Some(format!("Fill in {}", missing.join(", ")));
                        return Task::none();
                    }
                };
                if page.purpose == crate::preferences_page::Purpose::Arguments {
                    let id = page.command_id.clone();
                    self.page = Page::Root;
                    return match self.app_index.extensions().iter().position(|c| c.id == id) {
                        Some(index) => self.run_extension_command_with(index, Some(values)),
                        None => Task::none(),
                    };
                }
                let Some(backend) = self.backend.clone() else {
                    return Task::none();
                };
                let id = page.command_id.clone();
                Task::perform(
                    async move { backend.set_extension_preferences(id, values).await },
                    Message::PreferencesSaved,
                )
            }
            Message::PreferencesSaved(result) => {
                let Page::Preferences(page) = &mut self.page else {
                    return Task::none();
                };
                if let Err(reason) = result {
                    page.notice = Some(reason);
                    return Task::none();
                }
                let id = page.command_id.clone();
                let only_saved =
                    page.purpose == crate::preferences_page::Purpose::CommandPreferences;
                self.page = Page::Root;
                if only_saved {
                    if let Some(settings) = self.parked_settings.take() {
                        self.page = Page::Settings(settings);
                    }
                    return focus_search();
                }
                match self.app_index.extensions().iter().position(|c| c.id == id) {
                    Some(index) => self.run_extension_command(index),
                    None => Task::none(),
                }
            }
            _ => Task::none(),
        }
    }

    /// A checkbox in a form, ringed in the accent colour while the keyboard
    /// is on it: it cannot take Iced's focus, so it draws none of its own.
    pub(super) fn form_focus_ring<'a>(
        &self,
        focused: bool,
        field: Element<'a, Message>,
    ) -> Element<'a, Message> {
        let accent = self.palette().accent.to_iced();
        container(field)
            .padding(4)
            .style(move |_: &Theme| container::Style {
                border: Border {
                    color: if focused { accent } else { Color::TRANSPARENT },
                    width: 2.0,
                    radius: 6.0.into(),
                },
                ..container::Style::default()
            })
            .into()
    }

    pub(super) fn preferences_body<'a>(
        &'a self,
        page: &'a crate::preferences_page::PreferencesPage,
    ) -> Element<'a, Message> {
        use crate::backend::PreferenceInputKind;
        use crate::preferences_page::FieldValue;
        let mut form = column![
            iced::widget::text(match page.purpose {
                crate::preferences_page::Purpose::Preferences => {
                    format!("{} needs a few settings", page.title)
                }
                crate::preferences_page::Purpose::CommandPreferences => {
                    format!("{} settings", page.title)
                }
                crate::preferences_page::Purpose::Arguments
                | crate::preferences_page::Purpose::ShortcutArguments
                | crate::preferences_page::Purpose::ShortcutForm { .. }
                | crate::preferences_page::Purpose::SnippetArguments { .. }
                | crate::preferences_page::Purpose::SnippetForm { .. }
                | crate::preferences_page::Purpose::ScriptArguments
                | crate::preferences_page::Purpose::MediaArguments
                | crate::preferences_page::Purpose::GlyphKeywords
                | crate::preferences_page::Purpose::ClipboardKeywords
                | crate::preferences_page::Purpose::Alias
                | crate::preferences_page::Purpose::CreateExtension => page.title.clone(),
            })
            .font(self.font())
            .size(14)
        ]
        .spacing(12)
        .padding(Padding::new(16.0));
        for (index, (field, value)) in page.fields.iter().zip(&page.values).enumerate() {
            let label = if field.required {
                format!("{} *", field.title)
            } else {
                field.title.clone()
            };
            let mut entry =
                column![iced::widget::text(label).font(self.font()).size(13)].spacing(4);
            let input: Element<Message> = match (&field.kind, value) {
                (
                    PreferenceInputKind::Text | PreferenceInputKind::Password,
                    FieldValue::Text(text),
                ) => text_input(&field.placeholder, text)
                    .id(crate::preferences_page::field_id(index))
                    .secure(matches!(field.kind, PreferenceInputKind::Password))
                    .font(self.font())
                    .style(crate::adwaita::form_entry)
                    .on_input(move |text| Message::PreferenceEdited(index, FieldValue::Text(text)))
                    // No `on_submit`: the key handler submits the form on
                    // Enter, and `listen_with` delivers it whether or not the
                    // field captured it, so a field that also submitted made
                    // every Enter submit twice.
                    .padding(8)
                    .into(),
                (PreferenceInputKind::Checkbox { label }, FieldValue::Checked(checked)) => self
                    .form_focus_ring(
                        page.focus == Some(index),
                        iced::widget::checkbox(*checked)
                            .label(label.clone())
                            .on_toggle(move |checked| {
                                Message::PreferenceEdited(index, FieldValue::Checked(checked))
                            })
                            .into(),
                    ),
                (PreferenceInputKind::Dropdown { options }, FieldValue::Choice(choice)) => {
                    let titles: Vec<String> =
                        options.iter().map(|(title, _)| title.clone()).collect();
                    let selected = choice.as_ref().and_then(|value| {
                        options
                            .iter()
                            .find(|(_, v)| v == value)
                            .map(|(title, _)| title.clone())
                    });
                    let options = options.clone();
                    // Ringed through its own border, so it lines up with the
                    // text fields above it.
                    let ring = (page.focus == Some(index)).then(|| self.palette().accent.to_iced());
                    iced::widget::pick_list(titles, selected, move |title: String| {
                        let value = options
                            .iter()
                            .find(|(t, _)| *t == title)
                            .map(|(_, value)| value.clone());
                        Message::PreferenceEdited(index, FieldValue::Choice(value))
                    })
                    .font(self.font())
                    .menu_style({
                        let palette = self.palette();
                        move |_: &Theme| crate::design::dropdown_menu(palette)
                    })
                    .style(move |theme: &Theme, status| {
                        let mut style = crate::adwaita::form_dropdown(theme, status);
                        if let Some(accent) = ring {
                            style.border.color = accent;
                            style.border.width = 2.0;
                        }
                        style
                    })
                    .into()
                }
                (PreferenceInputKind::TextArea, _) => match page.editors.get(&index) {
                    Some(editor) => iced::widget::text_editor(editor)
                        .id(crate::preferences_page::field_id(index))
                        .placeholder(field.placeholder.as_str())
                        .font(self.font())
                        .height(Length::Fixed(120.0))
                        .padding(8)
                        .on_action(move |action| Message::PreferenceTextEdited(index, action))
                        .into(),
                    None => iced::widget::text("").into(),
                },
                (PreferenceInputKind::Unsupported { declared }, _) => {
                    iced::widget::text(crate::preferences_page::unsupported_note(declared))
                        .font(self.font())
                        .size(12)
                        .into()
                }
                _ => iced::widget::text("").into(),
            };
            entry = entry.push(input);
            if !field.description.is_empty() {
                entry = entry.push(
                    iced::widget::text(field.description.clone())
                        .font(self.font())
                        .size(11),
                );
            }
            form = form.push(entry);
        }
        form = form.push(
            iced::widget::text(page.purpose.hint())
                .font(self.font())
                .size(12),
        );
        if let Some(notice) = &page.notice {
            form = form.push(iced::widget::text(notice.clone()).font(self.font()));
        }
        scrollable(form)
            .id(crate::scroll::ROOT_RESULTS)
            .height(Length::Shrink)
            .into()
    }
}
