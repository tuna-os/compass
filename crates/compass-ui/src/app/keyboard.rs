//! The launcher's key handling: who gets a key press, in order.
//!
//! A question on screen takes Enter and Escape first, then the shortcut
//! recorders, the form a view is showing, the action panel while it is
//! open, and each builtin page's own keys; whatever is left moves the root
//! selection, launches, or is a quick-launch chord. Text typed into a field
//! is not handled here: it arrives through the field's `on_input`.

use super::{
    ClipboardChange, Direction, LauncherApp, Message, Page, Task, chord_direction, focus_search,
    form_focus, next_selection,
};

impl LauncherApp {
    /// A key event from the keyboard subscription.
    pub(super) fn keyboard_message(&mut self, message: Message) -> Task<Message> {
        match message {
            // The settings' shortcut recorder takes every key too.
            Message::Keyboard(event) if matches!(&self.page, Page::Settings(page) if page.recorder.is_some()) => {
                self.settings_recorder_event(&event)
            }
            // The shortcut recorder takes every key, releases included.
            Message::Keyboard(event)
                if self
                    .panel
                    .as_ref()
                    .is_some_and(|panel| panel.recorder.is_some()) =>
            {
                self.recorder_event(&event)
            }
            Message::Keyboard(iced::keyboard::Event::KeyPressed {
                ref key, modifiers, ..
            }) => {
                use iced::keyboard::{Key, key::Named};

                // A command's view takes every key the root list would, and
                // Escape goes back to the root rather than hiding the window:
                // one key undoes opening the wrong command.
                let panel_key = self.panel.is_some()
                    || (modifiers.control() && key.as_ref() == Key::Character("b"));
                if self.confirm.is_some() {
                    return match key.as_ref() {
                        Key::Named(Named::Enter) => self.run_confirmed(),
                        Key::Named(Named::Escape) => {
                            self.confirm = None;
                            focus_search()
                        }
                        _ => Task::none(),
                    };
                }
                if let Some(power) = self.power_confirm {
                    return match key.as_ref() {
                        Key::Named(Named::Enter) => self.run_power_command(power),
                        // Back to the search, as the generic confirmation
                        // above: the field lost the focus to the question.
                        Key::Named(Named::Escape) => {
                            self.power_confirm = None;
                            focus_search()
                        }
                        _ => Task::none(),
                    };
                }
                if matches!(self.page, Page::Onboarding(_)) {
                    return self.onboarding_key(key, modifiers);
                }
                if !panel_key && matches!(self.page, Page::StoreIntro(_)) {
                    return match key.as_ref() {
                        Key::Named(Named::Enter) => self.continue_to_store(),
                        Key::Named(Named::Escape) => self.update(Message::Back),
                        _ => crate::scroll::reading_key(key.as_ref()).unwrap_or_else(Task::none),
                    };
                }
                if let Page::Preferences(page) = &self.page {
                    return match key.as_ref() {
                        // A text area's Enter is a newline; the form submits
                        // with Ctrl+Enter.
                        Key::Named(Named::Enter)
                            if page.has_text_area() && !modifiers.control() =>
                        {
                            Task::none()
                        }
                        Key::Named(Named::Enter) => self.update(Message::PreferencesSubmit),
                        Key::Named(Named::Escape) => self.update(Message::Back),
                        Key::Named(Named::Tab) => {
                            let Page::Preferences(page) = &mut self.page else {
                                return Task::none();
                            };
                            page.step_focus(!modifiers.shift());
                            form_focus(page)
                        }
                        // A checkbox or a dropdown with the keyboard on it:
                        // Space ticks or steps, the arrows step.
                        _ if page.focus.is_some_and(|index| !page.is_text(index)) => {
                            use crate::preferences_page::FieldKey;
                            let field_key = match key.as_ref() {
                                Key::Named(Named::Space) => FieldKey::Toggle,
                                Key::Named(Named::ArrowDown) => FieldKey::Next,
                                Key::Named(Named::ArrowUp) => FieldKey::Previous,
                                _ => return Task::none(),
                            };
                            if let Page::Preferences(page) = &mut self.page {
                                page.field_key(field_key);
                            }
                            Task::none()
                        }
                        _ => Task::none(),
                    };
                }
                // An alert takes Enter and Escape and nothing else: the
                // extension is waiting on the answer.
                if let Page::Extension(page) = &mut self.page
                    && page.alert.is_some()
                {
                    let remembers = page
                        .alert
                        .as_ref()
                        .is_some_and(|alert| alert.remember_text.is_some());
                    let answer = match key.as_ref() {
                        Key::Named(Named::Enter) if remembers && modifiers.control() => None,
                        Key::Named(Named::Enter) => Some(true),
                        Key::Named(Named::Escape) => Some(false),
                        _ => return Task::none(),
                    };
                    page.alert = None;
                    let (session, backend) = (page.session, self.backend.clone());
                    let Some(backend) = backend else {
                        return Task::none();
                    };
                    return Task::perform(
                        async move {
                            match answer {
                                Some(confirmed) => {
                                    backend.extension_alert_answer(session, confirmed).await
                                }
                                None => backend.extension_alert_remember(session).await,
                            }
                        },
                        Message::ExtensionEventSent,
                    );
                }
                // A panel action's shortcut works whether or not the panel is
                // open: the panel says what the chord does, and the chord
                // should not need the panel to do it.
                if !panel_key && let Some(task) = self.advertised_chord(key, modifiers) {
                    return task;
                }
                if !panel_key
                    && let Page::Extension(page) = &self.page
                    && let Some(handler) = extension_chord(key, modifiers)
                        .and_then(|(mods, name)| page.action_for(&mods, &name))
                        .cloned()
                {
                    return self.extension_event(page.session, handler.0, page.action_args());
                }
                if !panel_key && let Page::Extension(page) = &mut self.page {
                    if page.form().is_some() && key.as_ref() == Key::Named(Named::Tab) {
                        return if modifiers.shift() {
                            iced::widget::operation::focus_previous()
                        } else {
                            iced::widget::operation::focus_next()
                        };
                    }
                    // A Markdown detail is read, not picked from: the
                    // arrows and the page keys scroll it.
                    if matches!(page.view, Some(compass_extension_api::View::Detail(_)))
                        && let Some(scroll) = crate::scroll::reading_key(key.as_ref())
                    {
                        return scroll;
                    }
                    let direction = match key.as_ref() {
                        Key::Named(Named::ArrowDown) => Some(Direction::Down),
                        Key::Named(Named::ArrowUp) => Some(Direction::Up),
                        Key::Named(Named::Escape) if page.depth > 1 => {
                            let session = page.session;
                            return self.extension_pop(session);
                        }
                        Key::Named(Named::Escape) => return self.update(Message::Back),
                        // A text area's Enter is a newline; its form submits
                        // with Ctrl+Enter.
                        Key::Named(Named::Enter)
                            if page.has_text_area() && !modifiers.control() =>
                        {
                            return Task::none();
                        }
                        Key::Named(Named::Enter) => return self.activate_extension_action(),
                        _ => chord_direction(self.keybinding, key.as_ref(), modifiers),
                    };
                    // A grid moves by cell and by row, as `SectionGridModel`.
                    if page.grid_columns.is_some() {
                        use crate::fonts_page::GridMove;
                        let step = match (key.as_ref(), direction) {
                            (Key::Named(Named::ArrowLeft), _) => Some(GridMove::Left),
                            (Key::Named(Named::ArrowRight), _) => Some(GridMove::Right),
                            (_, Some(Direction::Up)) => Some(GridMove::Up),
                            (_, Some(Direction::Down)) => Some(GridMove::Down),
                            _ => None,
                        };
                        if let Some(step) = step {
                            page.selected = page.grid_step(step, self.wrap_navigation);
                            return crate::scroll::reveal_root_selection();
                        }
                        return Task::none();
                    }
                    if let Some(direction) = direction {
                        page.selected = next_selection(
                            page.shown.len(),
                            page.selected,
                            direction,
                            self.wrap_navigation,
                        );
                        return crate::scroll::reveal_root_selection();
                    }
                    return Task::none();
                }
                if !panel_key && matches!(self.page, Page::Emoji(_)) {
                    return self.emoji_page_key(key, modifiers);
                }
                if !panel_key && let Some(task) = self.shortcut_chord(key, modifiers) {
                    return task;
                }
                if !panel_key && matches!(self.page, Page::Shortcuts(_)) {
                    return self.shortcuts_page_key(key, modifiers);
                }
                if !panel_key && matches!(self.page, Page::Snippets(_)) {
                    return self.snippets_page_key(key, modifiers);
                }
                if !panel_key && matches!(self.page, Page::ScriptOutput(_)) {
                    return self.script_output_key(key, modifiers);
                }
                if !panel_key && matches!(self.page, Page::Programs(_)) {
                    return self.programs_page_key(key, modifiers);
                }
                if !panel_key && matches!(self.page, Page::Dmenu(_)) {
                    return self.dmenu_page_key(key, modifiers);
                }
                if !panel_key && matches!(self.page, Page::Grants(_)) {
                    return self.grants_page_key(key, modifiers);
                }
                if !panel_key && matches!(self.page, Page::Tray(_)) {
                    return self.tray_page_key(key, modifiers);
                }
                if !panel_key && matches!(self.page, Page::Apps(_)) {
                    return self.apps_page_key(key, modifiers);
                }
                if !panel_key && matches!(self.page, Page::Calculator(_)) {
                    return self.calculator_page_key(key, modifiers);
                }
                if !panel_key && matches!(self.page, Page::Fallbacks(_)) {
                    return self.fallbacks_page_key(key, modifiers);
                }
                if !panel_key
                    && matches!(
                        self.page,
                        Page::Extensions(_) | Page::Icons(_) | Page::Storage(_) | Page::Tokens(_)
                    )
                {
                    return self.compass_view_key(key, modifiers);
                }
                if !panel_key && matches!(self.page, Page::Workspaces(_)) {
                    return self.workspaces_page_key(key, modifiers);
                }
                if !panel_key && matches!(self.page, Page::OpenWith(_)) {
                    return self.open_with_key(key, modifiers);
                }
                if !panel_key && matches!(self.page, Page::NowPlaying(_)) {
                    return self.now_playing_key(key, modifiers);
                }
                if !panel_key && matches!(self.page, Page::Themes(_)) {
                    return self.themes_page_key(key, modifiers);
                }
                if !panel_key && matches!(self.page, Page::Settings(_)) {
                    return self.settings_page_key(key, modifiers);
                }
                if !panel_key && matches!(self.page, Page::Created(_)) {
                    return self.created_page_key(key);
                }
                if !panel_key && matches!(self.page, Page::Fonts(_)) {
                    return self.fonts_page_key(key, modifiers);
                }
                if !panel_key && matches!(self.page, Page::FontPreview(_)) {
                    return self.font_preview_key(key);
                }
                if !panel_key && matches!(self.page, Page::Store(_)) {
                    return self.store_page_key(key, modifiers);
                }
                if !panel_key && matches!(self.page, Page::StoreDetail(_)) {
                    return self.store_detail_key(key);
                }
                if !panel_key && let Page::Files(page) = &mut self.page {
                    let direction = match key.as_ref() {
                        Key::Named(Named::ArrowDown) => Some(Direction::Down),
                        Key::Named(Named::ArrowUp) => Some(Direction::Up),
                        Key::Named(Named::Escape) => return self.update(Message::Back),
                        // Ctrl+Enter is `Keyboard::Shortcut::submit()`, which
                        // the C++ binds to "Show in file browser".
                        Key::Named(Named::Enter) => {
                            return self.open_selected_file(modifiers.control());
                        }
                        _ => chord_direction(self.keybinding, key.as_ref(), modifiers),
                    };
                    if let Some(direction) = direction {
                        page.selected = next_selection(
                            page.rows.len(),
                            page.selected,
                            direction,
                            self.wrap_navigation,
                        );
                        page.refresh_preview();
                        return crate::scroll::reveal_root_selection();
                    }
                    return Task::none();
                }
                if !panel_key && let Page::Windows(page) = &mut self.page {
                    if modifiers.control() && key.as_ref() == Key::Character("q") {
                        return self.close_selected_window();
                    }
                    let direction = match key.as_ref() {
                        Key::Named(Named::ArrowDown) => Some(Direction::Down),
                        Key::Named(Named::ArrowUp) => Some(Direction::Up),
                        Key::Named(Named::Escape) => return self.update(Message::Back),
                        Key::Named(Named::Enter) => return self.activate_selected_window(),
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
                    return Task::none();
                }
                if !panel_key && matches!(self.page, Page::Clipboard(_)) {
                    // The C++ defaults: `action.pin` is Ctrl+Shift+P and
                    // `action.remove` is Ctrl+X.
                    if let Key::Character(c) = key.as_ref()
                        && modifiers.control()
                        && !modifiers.alt()
                        && !modifiers.logo()
                    {
                        if modifiers.shift() && c.eq_ignore_ascii_case("p") {
                            return self
                                .change_selected_clipboard_entry(ClipboardChange::TogglePin);
                        }
                        if !modifiers.shift() && c.eq_ignore_ascii_case("x") {
                            return self.change_selected_clipboard_entry(ClipboardChange::Remove);
                        }
                    }
                    if let Some(task) = self.clipboard_chord(key, modifiers) {
                        return task;
                    }
                    let Page::Clipboard(page) = &mut self.page else {
                        return Task::none();
                    };
                    let direction = match key.as_ref() {
                        Key::Named(Named::ArrowDown) => Some(Direction::Down),
                        Key::Named(Named::ArrowUp) => Some(Direction::Up),
                        Key::Named(Named::Escape) => return self.update(Message::Back),
                        Key::Named(Named::Enter) => return self.paste_selected_clipboard_entry(),
                        _ => chord_direction(self.keybinding, key.as_ref(), modifiers),
                    };
                    if let Some(direction) = direction {
                        page.selected = next_selection(
                            page.rows.len(),
                            page.selected,
                            direction,
                            self.wrap_navigation,
                        );
                        return Task::batch([
                            crate::scroll::reveal_root_selection(),
                            self.clipboard_detail_task(),
                        ]);
                    }
                    return Task::none();
                }

                // Ctrl+B opens and closes the panel, over the list either way.
                //
                // Ctrl+B and not Ctrl+K, which is what the C++ binds on macOS
                // only: on Linux Ctrl+K is the vim chord for "move up", and
                // taking it here would have broken navigation for every user
                // of the default scheme. `keybind-manager.cpp` has the same
                // `#ifdef`, for the same reason.
                if modifiers.control() && key.as_ref() == Key::Character("b") {
                    return self.update(Message::TogglePanel);
                }

                // Ctrl+, opens the settings (`Keybind::OpenSettings`).
                if modifiers.control()
                    && self.panel.is_none()
                    && matches!(self.page, Page::Root)
                    && key.as_ref() == Key::Character(",")
                {
                    return self.open_settings(None);
                }

                // While the panel is open it takes the keys the list would
                // otherwise take. Escape closes the panel rather than the
                // launcher, because a panel opened by mistake should cost one
                // key and not the whole window.
                if self.panel.is_some() {
                    if let Some(task) = self.advertised_chord(key, modifiers) {
                        return task;
                    }
                    match key.as_ref() {
                        Key::Named(Named::ArrowDown) => {
                            return self.update(Message::PanelMove(Direction::Down));
                        }
                        Key::Named(Named::ArrowUp) => {
                            return self.update(Message::PanelMove(Direction::Up));
                        }
                        Key::Named(Named::Enter) => return self.update(Message::PanelActivate),
                        Key::Named(Named::Escape) => {
                            self.panel = None;
                            return focus_search();
                        }
                        _ => {}
                    }
                    return match chord_direction(self.keybinding, key.as_ref(), modifiers) {
                        Some(direction) => self.update(Message::PanelMove(direction)),
                        None => Task::none(),
                    };
                }

                // Ctrl+1..9 launches the Nth row outright (#87). Above the
                // arrows because it is a complete action rather than a
                // movement, and below the panel branch because while the panel
                // is open the list is not what a keystroke is aimed at.
                //
                // OUT OF RANGE DOES NOTHING, deliberately. `max_results` can be
                // below nine, and a query can match two things; Ctrl+7 then
                // refers to no row. Launching the last one instead would be a
                // guess at what the user meant, and the thing it launches is
                // whatever happens to be at the bottom of an unrelated list.
                //
                // An out-of-range position falls through rather than returning
                // early. The early return was there to stop the chord reaching
                // the navigation matcher, and a control could not be made to
                // fire on it: no scheme maps a digit to a direction, so the two
                // paths are indistinguishable. Rather than keep a branch no
                // test can tell apart from its absence, it is gone -- and if a
                // scheme ever does bind digits, the test that catches it is the
                // one that notices the selection moving.
                if self.quick_launch
                    && let Some(position) = quick_launch_position(key.as_ref(), modifiers)
                    && position < self.results.len()
                {
                    self.selected = position;
                    return self.update(Message::LaunchSelected);
                }

                // `ctrl+q` closes a window from the switcher (`CLOSE_WINDOW_SHORTCUT`).
                if modifiers.control()
                    && key.as_ref() == Key::Character("q")
                    && let Some(item) = self.selected_item()
                    && let Some(window_id) =
                        compass_core::window_switcher::window_launch_target_for_app(item.key())
                {
                    return self.update(Message::CloseWindow(window_id));
                }

                // Up at the top of the list reaches back through past searches.
                let up = match key.as_ref() {
                    Key::Named(Named::ArrowUp) => Some(Direction::Up),
                    _ => chord_direction(self.keybinding, key.as_ref(), modifiers),
                };
                if let Some(task) = self.history_up(up) {
                    return task;
                }
                let down = match key.as_ref() {
                    Key::Named(Named::ArrowDown) => Some(Direction::Down),
                    _ => chord_direction(self.keybinding, key.as_ref(), modifiers),
                };
                if let Some(task) = self.history_down(down) {
                    return task;
                }

                match key.as_ref() {
                    Key::Named(Named::ArrowDown) => {
                        return self.update(Message::MoveSelection(Direction::Down));
                    }
                    Key::Named(Named::ArrowUp) => {
                        return self.update(Message::MoveSelection(Direction::Up));
                    }
                    // Text in the field is cleared first, and only an empty
                    // field hides: `NavigationController::goBack` at the
                    // root search, as Raycast does. The field gives up its
                    // focus on Escape, so it is handed back for the next key.
                    Key::Named(Named::Escape) if !self.query.is_empty() => {
                        let cleared = self.update(Message::QueryChanged(String::new()));
                        return Task::batch([cleared, focus_search()]);
                    }
                    Key::Named(Named::Escape) => return self.update(Message::Dismiss),
                    // A message (a script's line, an error) stands in place
                    // of the results: Enter puts them back rather than
                    // launching a row nobody can see.
                    Key::Named(Named::Enter)
                        if self.error.is_some() && matches!(self.page, Page::Root) =>
                    {
                        self.error = None;
                        return focus_search();
                    }
                    Key::Named(Named::Enter) => return self.update(Message::LaunchSelected),
                    _ => {}
                }

                // A chord, if this one is. The scheme decides; `compass-core`
                // owns which chords each scheme has, so this front end does
                // not have a second opinion about it.
                match chord_direction(self.keybinding, key.as_ref(), modifiers) {
                    Some(direction) => self.update(Message::MoveSelection(direction)),
                    None => Task::none(),
                }
            }
            _ => Task::none(),
        }
    }
}

/// Which result `Ctrl+1`..`Ctrl+9` asks for, as a **position in the visible
/// list** counting from zero.
///
/// # The two things this is easy to get wrong
///
/// It returns a POSITION, not an item index. `LauncherApp::results` holds
/// indices into the application index, and the selection is a position within
/// `results` -- so `Ctrl+3` means the third row on screen, not item 3. Mixing
/// those up launches something plausible and wrong, which is the worst kind of
/// bug to have here because nothing looks broken.
///
/// And there is no zero. `Ctrl+1` is the first row, so the digit is one-based
/// and `Ctrl+0` is not a quick-launch chord at all.
///
/// Bounds are the caller's: this says which row was asked for, not whether it
/// exists.
#[must_use]
pub fn quick_launch_position(
    key: iced::keyboard::Key<&str>,
    modifiers: iced::keyboard::Modifiers,
) -> Option<usize> {
    use iced::keyboard::Key;

    // Control alone. A chord carrying Alt or Logo as well is somebody else's,
    // and claiming it would take a binding the desktop may already have.
    // Shift is excluded too: Ctrl+Shift+1 is a different chord, and on many
    // layouts it is not a digit at all.
    if !modifiers.control() || modifiers.alt() || modifiers.logo() || modifiers.shift() {
        return None;
    }

    let Key::Character(text) = key else {
        return None;
    };

    // `"1".."9"`, and nothing else: not "10", not a digit with anything
    // attached. `chars().next()` on a longer string would accept "1x".
    let mut chars = text.chars();
    let digit = chars.next()?;
    if chars.next().is_some() {
        return None;
    }
    let value = digit.to_digit(10)?;
    if value == 0 {
        return None;
    }
    Some(value as usize - 1)
}

pub(super) fn extension_chord(
    key: &iced::keyboard::Key,
    modifiers: iced::keyboard::Modifiers,
) -> Option<(Vec<compass_extension_api::action::KeyModifier>, String)> {
    use compass_extension_api::action::KeyModifier;
    use iced::keyboard::{Key, key::Named};
    if !(modifiers.control() || modifiers.alt() || modifiers.logo()) {
        return None;
    }
    let name = match key {
        Key::Character(c) => c.to_lowercase(),
        Key::Named(named) => match named {
            Named::Enter => "return",
            Named::Delete => "delete",
            Named::Backspace => "backspace",
            Named::Tab => "tab",
            Named::Space => "space",
            Named::ArrowUp => "arrowUp",
            Named::ArrowDown => "arrowDown",
            Named::ArrowLeft => "arrowLeft",
            Named::ArrowRight => "arrowRight",
            Named::Home => "home",
            Named::End => "end",
            Named::PageUp => "pageUp",
            Named::PageDown => "pageDown",
            _ => return None,
        }
        .to_owned(),
        Key::Unidentified => return None,
    };
    let mut mods = Vec::new();
    if modifiers.control() {
        mods.push(KeyModifier::Ctrl);
    }
    if modifiers.alt() {
        mods.push(KeyModifier::Alt);
    }
    if modifiers.shift() {
        mods.push(KeyModifier::Shift);
    }
    if modifiers.logo() {
        mods.push(KeyModifier::Meta);
    }
    Some((mods, name))
}
