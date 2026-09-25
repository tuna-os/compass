//! Snippet triggers: deciding when what someone typed should expand.
//!
//! Ports the matching half of `src/snippet/src/server.cpp`'s event loop. The
//! input server (`compass-input-server`) reads `/dev/input/event*` and turns
//! keycodes into text with xkbcommon; that is a device, not a decision. What
//! is a decision is the part below: given the text one key press produced,
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
//! character is still the `g`.
//!
//! # Longest trigger first, and the buffer empties on a match
//!
//! `createSnippet` keeps its list sorted by trigger length, longest first, and
//! the loop `break`s on the first match. So with `sig` and `;sig` both
//! registered, typing `;sig` expands `;sig` — whichever was registered first.
//! A match then clears the buffer, so the same characters cannot fire twice.
//!
//! # Every key press is checked, not just the ones that type
//!
//! The trigger loop runs for every press and repeat, including Backspace
//! (which xkb reports as `"\b"`) and keys that produce no text at all. The
//! one that matters: `;sigx` then Backspace leaves `;sig` in the buffer, and
//! the Backspace itself fires a keydown `;sig`.
//!
//! # The buffer is thirty-two bytes
//!
//! `MAX_BUFFER_SIZE` is 32 and the C++ keeps the *last* 32. A trigger longer
//! than that can never match. Only ASCII reaches the buffer (`std::isprint`
//! on a `char` in the C locale), so bytes and characters agree.

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

/// How many bytes of typing the matcher remembers.
///
/// `MAX_BUFFER_SIZE` in the C++. A trigger longer than this can never fire.
pub const MAX_BUFFER: usize = 32;

/// Whether `c` ends a word, as `isWordSeparator` decides it.
///
/// `isspace || ispunct`, over the C locale — so ASCII only.
#[must_use]
pub fn is_word_separator(c: char) -> bool {
    c.is_ascii_whitespace() || c == '\u{0b}' || c.is_ascii_punctuation()
}

/// Whether `c` is one `std::isprint` accepts in the C locale.
#[must_use]
pub const fn is_printable(c: char) -> bool {
    matches!(c, ' '..='~')
}

/// The typed-text buffer and the snippets watching it.
#[derive(Debug, Clone, Default)]
pub struct Matcher {
    snippets: Vec<Snippet>,
    text: String,
}

impl Matcher {
    /// A matcher with no snippets.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a snippet, keeping the list longest trigger first.
    ///
    /// The C++ sorts with `std::ranges::sort`, which leaves the order of two
    /// triggers of the same length unspecified; this sort is stable, so the
    /// earlier registration wins a tie. The same trigger may be registered
    /// twice, as the C++ allows; the duplicate never fires, the first does.
    pub fn add(&mut self, snippet: Snippet) {
        self.snippets.push(snippet);
        self.snippets
            .sort_by_key(|snippet| std::cmp::Reverse(snippet.trigger.len()));
    }

    /// Removes every snippet with `trigger`, answering whether one went.
    pub fn remove(&mut self, trigger: &str) -> bool {
        let before = self.snippets.len();
        self.snippets.retain(|s| s.trigger != trigger);
        self.snippets.len() != before
    }

    /// The registered snippets, in matching order.
    #[must_use]
    pub fn snippets(&self) -> &[Snippet] {
        &self.snippets
    }

    /// Forgets what has been typed, as `resetContext` does.
    ///
    /// Called when the focused window changes: text typed in one place must
    /// not complete a trigger in another.
    pub fn reset(&mut self) {
        self.text.clear();
    }

    /// What has been typed, up to [`MAX_BUFFER`] bytes.
    #[must_use]
    pub fn buffer(&self) -> &str {
        &self.text
    }

    /// Feeds the text one key press (or repeat) produced, and answers the
    /// trigger that fired, if one did.
    ///
    /// `text` is what xkb says the key types — empty for a modifier, `"\b"`
    /// for Backspace. `backspace` is whether the key was `KEY_BACKSPACE`,
    /// which erases the last byte instead of adding one.
    pub fn on_key(&mut self, text: &str, backspace: bool) -> Option<String> {
        let first = text.chars().next();

        if let Some(first) = first {
            if backspace {
                self.text.pop();
            } else if is_printable(first) {
                self.text.push_str(text);
                if self.text.len() > MAX_BUFFER {
                    let excess = self.text.len() - MAX_BUFFER;
                    // ASCII only reaches here in practice; a boundary check
                    // keeps a multi-byte key text from splitting a character.
                    let cut = (excess..=self.text.len())
                        .find(|&i| self.text.is_char_boundary(i))
                        .unwrap_or(self.text.len());
                    self.text.drain(..cut);
                }
            }
        }

        let separator = first.is_some_and(is_word_separator);

        let fired = self.snippets.iter().find(|snippet| match snippet.mode {
            ExpansionMode::Keydown => self.text.ends_with(&snippet.trigger),
            ExpansionMode::Word => {
                separator
                    && snippet.trigger.len() < self.text.len()
                    && self.text.is_char_boundary(self.text.len() - 1)
                    && self.text[..self.text.len() - 1].ends_with(&snippet.trigger)
            }
        })?;

        let trigger = fired.trigger.clone();
        self.text.clear();
        Some(trigger)
    }

    /// [`Self::on_key`] for a key that types `c`.
    pub fn on_char(&mut self, c: char) -> Option<String> {
        let mut buf = [0u8; 4];
        self.on_key(c.encode_utf8(&mut buf), false)
    }

    /// [`Self::on_key`] for Backspace.
    pub fn on_backspace(&mut self) -> Option<String> {
        self.on_key("\u{8}", true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Types `text` and returns every trigger it fired.
    fn type_text(matcher: &mut Matcher, text: &str) -> Vec<String> {
        text.chars().filter_map(|c| matcher.on_char(c)).collect()
    }

    #[test]
    fn a_keydown_snippet_fires_on_the_last_character_of_its_trigger() {
        let mut matcher = Matcher::new();
        matcher.add(Snippet::new(";sig", ExpansionMode::Keydown));

        assert_eq!(type_text(&mut matcher, ";si"), Vec::<String>::new());
        assert_eq!(matcher.on_char('g').as_deref(), Some(";sig"));
    }

    #[test]
    fn a_word_snippet_waits_for_a_separator_and_does_not_eat_it() {
        let mut matcher = Matcher::new();
        matcher.add(Snippet::new(";sig", ExpansionMode::Word));

        assert!(
            type_text(&mut matcher, ";sig").is_empty(),
            "a word snippet must not fire on the trigger alone"
        );
        assert_eq!(matcher.on_char(' ').as_deref(), Some(";sig"));
    }

    #[test]
    fn punctuation_ends_a_word_but_a_tab_does_not() {
        for separator in ['.', ',', '!', ')', ' '] {
            let mut matcher = Matcher::new();
            matcher.add(Snippet::new(";sig", ExpansionMode::Word));
            let _ = type_text(&mut matcher, ";sig");
            assert_eq!(
                matcher.on_char(separator).as_deref(),
                Some(";sig"),
                "{separator:?} should have ended the word"
            );
        }

        for invisible in ['\t', '\n', '\r'] {
            let mut matcher = Matcher::new();
            matcher.add(Snippet::new(";sig", ExpansionMode::Word));
            let _ = type_text(&mut matcher, ";sig");
            assert_eq!(
                matcher.on_char(invisible),
                None,
                "{invisible:?} expanded; it is a separator but never reaches the buffer"
            );
            assert!(is_word_separator(invisible));
        }

        assert!(!is_word_separator('a'));
        assert!(!is_word_separator('1'));
    }

    #[test]
    fn a_word_snippet_does_not_fire_inside_a_longer_word() {
        let mut matcher = Matcher::new();
        matcher.add(Snippet::new(";sig", ExpansionMode::Word));
        assert!(type_text(&mut matcher, ";sigg ").is_empty());
    }

    #[test]
    fn a_backspace_that_leaves_a_trigger_behind_fires_it() {
        // The trigger loop runs on Backspace too: xkb gives it the text "\b",
        // the byte is erased, and the buffer ending in `;sig` is checked.
        // Reachable when the snippet is registered while the text is typed
        // (the buffer cannot otherwise end in a keydown trigger unfired).
        let mut matcher = Matcher::new();
        let _ = type_text(&mut matcher, ";sigx");
        matcher.add(Snippet::new(";sig", ExpansionMode::Keydown));
        assert_eq!(matcher.on_backspace().as_deref(), Some(";sig"));

        // A word snippet does not: `"\b"` is neither space nor punctuation.
        let mut matcher = Matcher::new();
        matcher.add(Snippet::new(";sig", ExpansionMode::Word));
        let _ = type_text(&mut matcher, ";sigx");
        assert_eq!(matcher.on_backspace(), None);
        assert_eq!(matcher.buffer(), ";sig");
    }

    #[test]
    fn backspace_erases_one_byte() {
        let mut matcher = Matcher::new();
        let _ = type_text(&mut matcher, "abc");
        assert_eq!(matcher.on_backspace(), None);
        assert_eq!(matcher.buffer(), "ab");
        let _ = matcher.on_backspace();
        let _ = matcher.on_backspace();
        let _ = matcher.on_backspace();
        assert_eq!(matcher.buffer(), "", "an empty buffer stays empty");
    }

    #[test]
    fn a_match_empties_the_buffer() {
        let mut matcher = Matcher::new();
        matcher.add(Snippet::new("aa", ExpansionMode::Keydown));
        assert_eq!(type_text(&mut matcher, "aaa"), vec!["aa".to_owned()]);
        assert_eq!(matcher.buffer(), "a", "the third `a` starts afresh");
        assert_eq!(matcher.on_char('a').as_deref(), Some("aa"));
    }

    #[test]
    fn the_buffer_holds_the_last_thirty_two_bytes_and_no_more() {
        let mut matcher = Matcher::new();
        let _ = type_text(&mut matcher, &"a".repeat(100));
        assert_eq!(matcher.buffer(), "a".repeat(MAX_BUFFER));
    }

    #[test]
    fn a_trigger_longer_than_the_buffer_can_never_fire() {
        let trigger = "x".repeat(MAX_BUFFER + 1);
        let mut matcher = Matcher::new();
        matcher.add(Snippet::new(trigger.clone(), ExpansionMode::Keydown));
        assert!(type_text(&mut matcher, &trigger).is_empty());
    }

    #[test]
    fn the_longest_trigger_wins_whatever_the_registration_order() {
        for order in [["sig", ";sig"], [";sig", "sig"]] {
            let mut matcher = Matcher::new();
            for trigger in order {
                matcher.add(Snippet::new(trigger, ExpansionMode::Keydown));
            }
            assert_eq!(type_text(&mut matcher, ";sig"), vec![";sig".to_owned()]);
        }
    }

    #[test]
    fn resetting_forgets_what_was_typed() {
        let mut matcher = Matcher::new();
        matcher.add(Snippet::new(";sig", ExpansionMode::Keydown));

        let _ = type_text(&mut matcher, ";si");
        matcher.reset();
        assert_eq!(matcher.on_char('g'), None);
        assert_eq!(matcher.buffer(), "g");
    }

    #[test]
    fn removing_a_snippet_stops_it_firing() {
        let mut matcher = Matcher::new();
        matcher.add(Snippet::new(";sig", ExpansionMode::Keydown));
        matcher.add(Snippet::new(";sig", ExpansionMode::Keydown));
        assert!(matcher.remove(";sig"), "both copies go");
        assert!(!matcher.remove(";sig"));
        assert!(type_text(&mut matcher, ";sig").is_empty());
    }

    #[test]
    fn non_printing_and_non_ascii_text_does_not_reach_the_buffer() {
        let mut matcher = Matcher::new();
        let _ = matcher.on_char('\u{1b}');
        let _ = matcher.on_char('é');
        let _ = matcher.on_key("", false);
        assert_eq!(matcher.buffer(), "");
    }
}
