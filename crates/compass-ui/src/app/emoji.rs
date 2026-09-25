//! The emoji picker in the launcher: its keys, its action panel and the
//! keyword form, over [`crate::emoji_page`].

use iced::keyboard::{Key, Modifiers, key::Named};

use super::{
    Direction, LauncherApp, Message, Page, PanelState, Task, chord_direction, focus_search,
    next_selection,
};
use crate::backend::{PreferenceInput, PreferenceInputKind};
use crate::emoji_page::{EmojiPage, actions};
use crate::preferences_page::{FieldValue, PreferencesPage, Purpose};

/// `EditEmojiKeywordsAction`'s description, under the field.
const KEYWORDS_DESCRIPTION: &str = "Additional keywords that will be used to index this glyph";

impl LauncherApp {
    /// Opens Search Emojis & Symbols over what the person has done with it.
    pub(super) fn open_emoji_picker(&mut self) -> Task<Message> {
        self.page = Page::Emoji(EmojiPage::open(
            self.glyph_path.clone(),
            self.emoji_skin_tone.as_deref(),
        ));
        focus_search()
    }

    /// The picker's keys, below the panel's.
    pub(super) fn emoji_page_key(&mut self, key: &Key, modifiers: Modifiers) -> Task<Message> {
        if modifiers.control() && key.as_ref() == Key::Character("e") {
            return self.edit_emoji_keywords();
        }
        let Page::Emoji(page) = &mut self.page else {
            return Task::none();
        };
        let direction = match key.as_ref() {
            Key::Named(Named::ArrowDown) => Some(Direction::Down),
            Key::Named(Named::ArrowUp) => Some(Direction::Up),
            Key::Named(Named::Escape) => return self.update(Message::Back),
            Key::Named(Named::Enter) => return self.copy_selected_emoji(),
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

    /// Copies the selected glyph in its tone, counts the pick, and gets out
    /// of the way so it can be pasted where the person was.
    pub(super) fn copy_selected_emoji(&mut self) -> Task<Message> {
        let Page::Emoji(page) = &mut self.page else {
            return Task::none();
        };
        let Some(glyph) = page.selected_glyph() else {
            return Task::none();
        };
        let text = page.display(glyph);
        page.register_visit(glyph);
        self.panel = None;
        Task::batch([iced::clipboard::write(text), self.conceal()])
    }

    /// The panel over the selected glyph.
    pub(super) fn open_emoji_panel(&mut self) -> Option<Task<Message>> {
        let Page::Emoji(page) = &self.page else {
            return None;
        };
        let sections = page.panel_sections();
        if sections.is_empty() {
            return Some(Task::none());
        }
        self.panel = Some(PanelState::new(sections));
        Some(iced::widget::operation::focus(super::PANEL_INPUT))
    }

    /// Runs an emoji panel action, if `id` is one.
    pub(super) fn emoji_panel_action(&mut self, id: &str) -> Option<Task<Message>> {
        let Page::Emoji(page) = &mut self.page else {
            return None;
        };
        let glyph = page.selected_glyph()?;
        let copy = |text: String| Task::batch([iced::clipboard::write(text), focus_search()]);
        let task = match id {
            actions::COPY => return Some(self.copy_selected_emoji()),
            actions::COPY_NAME => copy(glyph.name.to_owned()),
            actions::COPY_CODEPOINT => copy(compass_core::emoji_grid::formatted_codepoint(
                glyph.character,
            )),
            actions::COPY_CATEGORY => copy(glyph.category.label().to_owned()),
            actions::EDIT_KEYWORD => return Some(self.edit_emoji_keywords()),
            actions::RESET_RANKING => {
                page.reset_ranking(glyph);
                focus_search()
            }
            actions::PIN | actions::UNPIN => {
                page.toggle_pin(glyph);
                Task::batch([focus_search(), crate::scroll::reveal_root_selection()])
            }
            actions::TONE_RESET => {
                page.set_tone(glyph, None);
                focus_search()
            }
            other => {
                let tone = other
                    .strip_prefix(actions::TONE)
                    .and_then(compass_core::emoji_grid::skin_tone_by_id)?;
                page.set_tone(glyph, Some(tone));
                focus_search()
            }
        };
        self.panel = None;
        Some(task)
    }

    /// Opens the keyword form for the selected glyph, keeping the picker to
    /// come back to.
    fn edit_emoji_keywords(&mut self) -> Task<Message> {
        let Page::Emoji(page) = &self.page else {
            return Task::none();
        };
        let Some(glyph) = page.selected_glyph() else {
            return Task::none();
        };
        let field = PreferenceInput {
            name: "keywords".to_owned(),
            title: "Keywords".to_owned(),
            description: KEYWORDS_DESCRIPTION.to_owned(),
            placeholder: String::new(),
            required: false,
            kind: PreferenceInputKind::Text,
            value: Some(serde_json::Value::String(page.keywords(glyph))),
        };
        let form = PreferencesPage::new(
            Purpose::GlyphKeywords,
            glyph.character.to_owned(),
            "Edit keyword".to_owned(),
            vec![field],
        );
        self.panel = None;
        let Page::Emoji(page) = std::mem::replace(&mut self.page, Page::Root) else {
            return Task::none();
        };
        self.parked_emoji = Some(page);
        self.page = Page::Preferences(Box::new(form));
        iced::widget::operation::focus_next()
    }

    /// Saves the keyword form and goes back to the picker. `None` when the
    /// form showing is not the keyword form.
    pub(super) fn submit_emoji_keywords(&mut self) -> Option<Task<Message>> {
        let Page::Preferences(form) = &self.page else {
            return None;
        };
        if form.purpose != Purpose::GlyphKeywords {
            return None;
        }
        let character = form.command_id.clone();
        let keywords = match form.values.first() {
            Some(FieldValue::Text(text)) => text.clone(),
            _ => String::new(),
        };
        let mut page = self.parked_emoji.take().unwrap_or_default();
        page.set_keywords(&character, &keywords);
        self.page = Page::Emoji(page);
        Some(focus_search())
    }

    /// Escape from the keyword form: back to the picker, as it was.
    pub(super) fn back_from_emoji_keywords(&mut self) -> Option<Task<Message>> {
        let Page::Preferences(form) = &self.page else {
            return None;
        };
        if form.purpose != Purpose::GlyphKeywords {
            return None;
        }
        self.page = Page::Emoji(self.parked_emoji.take().unwrap_or_default());
        Some(focus_search())
    }
}
