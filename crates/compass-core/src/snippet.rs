//! Snippet triggers: deciding when what someone typed should expand.
//!
//! Ports the matching half of `src/snippet/src/server.cpp`. The C++ server
//! reads `/dev/input/event*` through libudev and turns keycodes into text with
//! xkbcommon; none of that is here, because none of it is a decision. What is
//! a decision is the part below: given the characters that have been typed,
//! has a trigger fired?
//!
//! # The two modes are not the same rule with a flag
//!
//! * **`Keydown`** fires the moment the buffer ends with the trigger. `;sig`
//!   expands as soon as the `g` is typed.
//! * **`Word`** fires when a word separator is typed *after* the trigger, and
//!   the separator itself is not part of the match — the C++ checks
//!   `m_text.substr(0, m_text.size() - 1).ends_with(trigger)`. So `;sig ` with
//!   a trailing space expands, and `;sigg` does not.
//!
//! A separator is `isspace(c) || ispunct(c)`, which is what makes `;sig.`
//! expand as well as `;sig `. A **tab does not expand**, and neither does a
//! newline: the buffer only takes characters `isprint` accepts, so the tab
//! never enters it, and the word check then runs against a buffer whose last
//! character is still the `g`. Both engines behave that way, and a port that
//! "fixed" it would expand where the other does not.
//!
//! # The buffer is thirty-two characters, and that is a rule too
//!
//! `MAX_BUFFER_SIZE` is 32 and the C++ keeps the *last* 32 characters. A
//! trigger longer than that can never match, and a long line of typing cannot
//! grow the buffer without bound. Both matter, and the second is why this is
//! not a `String` that grows.
//!
//! # What is deliberately not here
//!
//! The wire protocol. `src/snippet/src/server.cpp` frames with
//! `*reinterpret_cast<uint32_t *>(data.data())` — a **native-endian** length,
//! unlike the extension worker's big-endian one, and unlike anything a
//! remote peer could rely on. A port of the transport has to decide what to do
//! about that; a port of the matcher does not, so this does not pretend to.

/// When a snippet expands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExpansionMode {
    /// As soon as the trigger has been typed.
    Keydown,
    /// When a word separator follows the trigger.
    Word,
}

/// A registered snippet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snippet {
    /// The text that fires it.
    pub trigger: String,
    /// When it fires.
    pub mode: ExpansionMode,
}

impl Snippet {
    /// A snippet with a trigger and a mode.
    #[must_use]
    pub fn new(trigger: impl Into<String>, mode: ExpansionMode) -> Self {
        Self {
            trigger: trigger.into(),
            mode,
        }
    }
}

/// How many characters of typing the matcher remembers.
///
/// `MAX_BUFFER_SIZE` in the C++. A trigger longer than this can never fire.
pub const MAX_BUFFER: usize = 32;

/// Whether `c` ends a word, as `isWordSeparator` decides it.
///
/// `isspace || ispunct`, over the C locale — so ASCII only, which is what the
/// C++ `std::isspace`/`std::ispunct` on a `char` gives.
#[must_use]
pub fn is_word_separator(c: char) -> bool {
    c.is_ascii_whitespace() || (c.is_ascii_punctuation())
}

/// What a keystroke did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// Nothing.
    None,
    /// This snippet should expand.
    Expand {
        /// Which trigger fired.
        trigger: String,
    },
    /// The last expansion should be undone.
    ///
    /// The C++ arms this after every expansion and fires it if the *next* key
    /// is a backspace: typing a trigger by accident is undone by the reflex
    /// that undoes any typo.
    Undo {
        /// Which trigger is being undone.
        trigger: String,
    },
}

/// The typed-text buffer and the snippets watching it.
#[derive(Debug, Clone, Default)]
pub struct Matcher {
    snippets: Vec<Snippet>,
    text: String,
    undo_trigger: Option<String>,
}

impl Matcher {
    /// A matcher with no snippets.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a snippet.
    pub fn add(&mut self, snippet: Snippet) {
        self.snippets.push(snippet);
    }

    /// Removes a snippet by trigger, answering whether one went.
    pub fn remove(&mut self, trigger: &str) -> bool {
        let before = self.snippets.len();
        self.snippets.retain(|s| s.trigger != trigger);
        self.snippets.len() != before
    }

    /// Forgets what has been typed, as `resetContext` does.
    ///
    /// Called when the focused window changes: text typed in one place must
    /// not complete a trigger in another.
    pub fn reset(&mut self) {
        self.text.clear();
        self.undo_trigger = None;
    }

    /// What has been typed, up to [`MAX_BUFFER`] characters.
    #[must_use]
    pub fn buffer(&self) -> &str {
        &self.text
    }

    /// Feeds one printable character.
    #[must_use]
    pub fn on_char(&mut self, c: char) -> Event {
        self.undo_trigger = None;

        if c.is_ascii_graphic() || c == ' ' {
            self.text.push(c);
            while self.text.chars().count() > MAX_BUFFER {
                let first = self.text.chars().next().map(char::len_utf8).unwrap_or(0);
                self.text.drain(..first);
            }
        }

        let separator = is_word_separator(c);
        self.check(separator)
    }

    /// Feeds a backspace.
    ///
    /// A backspace immediately after an expansion is the undo; otherwise it
    /// erases one character, and never fires a trigger.
    #[must_use]
    pub fn on_backspace(&mut self) -> Event {
        if let Some(trigger) = self.undo_trigger.take() {
            return Event::Undo { trigger };
        }
        self.text.pop();
        Event::None
    }

    /// The first snippet that matches, if any.
    fn check(&mut self, separator: bool) -> Event {
        for snippet in &self.snippets {
            let fired = match snippet.mode {
                ExpansionMode::Keydown => {
                    snippet.trigger.len() <= self.text.len()
                        && self.text.ends_with(&snippet.trigger)
                }
                ExpansionMode::Word => {
                    // `snippet.trigger.size() + 1 > m_text.size()` in the C++,
                    // written the other way round here because clippy reads
                    // the `+ 1` as a fencepost. Same comparison: the trigger
                    // plus the separator has to fit.
                    separator
                        && snippet.trigger.len() < self.text.len()
                        && self.text[..self.text.len() - 1].ends_with(&snippet.trigger)
                }
            };

            if fired {
                let trigger = snippet.trigger.clone();
                self.undo_trigger = Some(trigger.clone());
                return Event::Expand { trigger };
            }
        }
        Event::None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    const CPP: &str = "src/snippet/src/server.cpp";

    fn read_cpp() -> String {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("two levels below the repository root")
            .join(CPP);
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
    }

    /// Types `text` and returns every event it produced.
    fn type_text(matcher: &mut Matcher, text: &str) -> Vec<Event> {
        text.chars().map(|c| matcher.on_char(c)).collect()
    }

    fn expansions(events: &[Event]) -> Vec<&str> {
        events
            .iter()
            .filter_map(|event| match event {
                Event::Expand { trigger } => Some(trigger.as_str()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_keydown_snippet_fires_on_the_last_character_of_its_trigger() {
        let mut matcher = Matcher::new();
        matcher.add(Snippet::new(";sig", ExpansionMode::Keydown));

        let events = type_text(&mut matcher, ";sig");
        assert_eq!(expansions(&events), vec![";sig"]);
        assert_eq!(
            events.len(),
            4,
            "one event per keystroke, whatever they say"
        );
    }

    #[test]
    fn a_word_snippet_waits_for_a_separator_and_does_not_eat_it() {
        // `m_text.substr(0, m_text.size() - 1).ends_with(trigger)`: the
        // separator is typed, then the match is checked against everything
        // before it.
        let mut matcher = Matcher::new();
        matcher.add(Snippet::new(";sig", ExpansionMode::Word));

        assert!(
            expansions(&type_text(&mut matcher, ";sig")).is_empty(),
            "a word snippet must not fire on the trigger alone"
        );
        assert_eq!(expansions(&[matcher.on_char(' ')]), vec![";sig"]);
    }

    #[test]
    fn punctuation_ends_a_word_but_a_tab_does_not() {
        // Two rules meeting. `isWordSeparator` is `isspace || ispunct`, so a
        // tab *is* a separator -- but the buffer only takes characters
        // `isprint` accepts, and a tab is not one. The word check then runs
        // against a buffer the tab never entered, and `;sig` minus its last
        // character is `;si`, which is not the trigger. So a tab does not
        // expand in either engine, and neither does a newline.
        for separator in ['.', ',', '!', ')', ' '] {
            let mut matcher = Matcher::new();
            matcher.add(Snippet::new(";sig", ExpansionMode::Word));
            let _ = type_text(&mut matcher, ";sig");
            assert_eq!(
                expansions(&[matcher.on_char(separator)]),
                vec![";sig"],
                "{separator:?} should have ended the word"
            );
        }

        for invisible in ['\t', '\n'] {
            let mut matcher = Matcher::new();
            matcher.add(Snippet::new(";sig", ExpansionMode::Word));
            let _ = type_text(&mut matcher, ";sig");
            assert!(
                expansions(&[matcher.on_char(invisible)]).is_empty(),
                "{invisible:?} expanded; it is a separator but never reaches the buffer"
            );
            assert!(
                is_word_separator(invisible),
                "{invisible:?} should still be a separator by itself"
            );
        }

        assert!(!is_word_separator('a'), "a letter is not a separator");
        assert!(!is_word_separator('1'), "a digit is not a separator");
    }

    #[test]
    fn a_word_snippet_does_not_fire_when_the_trigger_is_part_of_a_longer_word() {
        let mut matcher = Matcher::new();
        matcher.add(Snippet::new(";sig", ExpansionMode::Word));
        let events = type_text(&mut matcher, ";sigg ");
        assert!(
            expansions(&events).is_empty(),
            "`;sigg ` is not `;sig` followed by a separator"
        );
    }

    #[test]
    fn backspace_erases_and_never_fires() {
        let mut matcher = Matcher::new();
        matcher.add(Snippet::new(";sig", ExpansionMode::Keydown));

        let _ = type_text(&mut matcher, ";sigx");
        assert_eq!(matcher.buffer(), ";sigx");

        // The backspace brings the buffer back to `;sig`, and the C++ checks
        // triggers only on a key that produced text -- a backspace produces
        // none, so nothing fires.
        assert_eq!(matcher.on_backspace(), Event::None);
        assert_eq!(matcher.buffer(), ";sig");
    }

    #[test]
    fn a_backspace_straight_after_an_expansion_is_an_undo() {
        let mut matcher = Matcher::new();
        matcher.add(Snippet::new(";sig", ExpansionMode::Keydown));
        let _ = type_text(&mut matcher, ";sig");

        assert_eq!(
            matcher.on_backspace(),
            Event::Undo {
                trigger: ";sig".to_owned()
            },
            "the reflex that undoes a typo has to undo an accidental expansion"
        );

        // And only straight after: one more character disarms it.
        let _ = type_text(&mut matcher, ";sig");
        let _ = matcher.on_char('x');
        assert_eq!(matcher.on_backspace(), Event::None);
    }

    #[test]
    fn the_buffer_holds_the_last_thirty_two_characters_and_no_more() {
        let cpp = read_cpp();
        assert!(
            cpp.contains("MAX_BUFFER_SIZE = 32"),
            "{CPP} no longer keeps 32 characters; this port's buffer would disagree"
        );

        let mut matcher = Matcher::new();
        let _ = type_text(&mut matcher, &"a".repeat(100));
        assert_eq!(matcher.buffer().chars().count(), MAX_BUFFER);
        assert_eq!(matcher.buffer(), "a".repeat(MAX_BUFFER));
    }

    #[test]
    fn a_trigger_longer_than_the_buffer_can_never_fire() {
        // Not a bug to fix here: the C++ cannot match it either, and a port
        // that could would expand where the other engine does not.
        let trigger = "x".repeat(MAX_BUFFER + 1);
        let mut matcher = Matcher::new();
        matcher.add(Snippet::new(trigger.clone(), ExpansionMode::Keydown));

        let events = type_text(&mut matcher, &trigger);
        assert!(expansions(&events).is_empty());
    }

    #[test]
    fn the_first_registered_snippet_wins() {
        // `break` on the first match: two snippets whose triggers both end the
        // buffer are resolved by registration order, not by length.
        let mut matcher = Matcher::new();
        matcher.add(Snippet::new("sig", ExpansionMode::Keydown));
        matcher.add(Snippet::new(";sig", ExpansionMode::Keydown));

        assert_eq!(expansions(&type_text(&mut matcher, ";sig")), vec!["sig"]);
    }

    #[test]
    fn resetting_forgets_what_was_typed() {
        // `resetContext` is called when focus moves. Text typed in one window
        // completing a trigger in another is the bug this prevents.
        let mut matcher = Matcher::new();
        matcher.add(Snippet::new(";sig", ExpansionMode::Keydown));

        let _ = type_text(&mut matcher, ";si");
        matcher.reset();
        assert!(expansions(&[matcher.on_char('g')]).is_empty());
        assert_eq!(matcher.buffer(), "g");
    }

    #[test]
    fn removing_a_snippet_stops_it_firing() {
        let mut matcher = Matcher::new();
        matcher.add(Snippet::new(";sig", ExpansionMode::Keydown));
        assert!(matcher.remove(";sig"));
        assert!(!matcher.remove(";sig"), "removing twice removes nothing");

        assert!(expansions(&type_text(&mut matcher, ";sig")).is_empty());
    }

    #[test]
    fn a_non_printing_character_does_not_reach_the_buffer() {
        // The C++ appends only when `std::isprint(keyStr.at(0))`.
        let mut matcher = Matcher::new();
        let _ = matcher.on_char('\u{1b}');
        assert_eq!(matcher.buffer(), "", "an escape reached the buffer");
    }
}
