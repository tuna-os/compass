//! Turning a script's output into styled text.
//!
//! Ports `ScriptOutputTokenizer`. A script command's stdout is arbitrary bytes
//! from someone else's program, and the launcher renders it: this is what
//! decides which parts of it are links, which are coloured, and which are
//! neither.

/// A colour the output can ask for.
///
/// The palette is the theme's rather than the terminal's — a script asking for
/// red gets the theme's red, so its output matches the window it is drawn in
/// instead of the terminal it was written for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Color {
    /// `SemanticColor::Red`.
    Red,
    /// `SemanticColor::Green`.
    Green,
    /// `SemanticColor::Yellow`.
    Yellow,
    /// `SemanticColor::Blue`.
    Blue,
    /// `SemanticColor::Magenta`.
    Magenta,
    /// `SemanticColor::Cyan`.
    Cyan,
    /// Plain black, not a theme colour.
    Black,
    /// Whatever the theme draws ordinary text in.
    TextPrimary,
}

/// What an escape sequence asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Format {
    /// The foreground colour, if one was named.
    pub foreground: Option<Color>,
    /// The background colour, if one was named.
    pub background: Option<Color>,
    /// Whether the sequence included a reset.
    pub reset: bool,
}

/// One run of output.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Token {
    /// The text of the run.
    pub text: String,
    /// Whether the run is a link.
    pub url: bool,
    /// The formatting that applies from here on, if this run set any.
    pub fmt: Option<Format>,
}

/// The schemes that start a link.
///
/// `http` and `https` only, and matched as written — a script printing
/// `ftp://` gets text, which is the safer answer for something the launcher
/// will make clickable.
pub const URL_SCHEMES: &[&str] = &["http://", "https://"];

/// Whether a character continues a link.
///
/// Quotes and parentheses end it, because a URL printed inside them — which is
/// how most prose prints one — would otherwise swallow the closing mark and
/// produce a link that 404s.
#[must_use]
pub fn is_valid_url_char(ch: char) -> bool {
    !ch.is_control() && !ch.is_whitespace() && !matches!(ch, '"' | '\'' | '(' | ')')
}

/// Which colour an ANSI foreground code names.
///
/// Only 30–37 ever reach here: the background codes are shifted down by ten and
/// the bright ones by sixty before the lookup. **37 is not in the table**, so
/// white text draws in the theme's ordinary foreground — which is what makes a
/// script that colours its output white readable on a light theme.
///
/// The C++ switch also has arms for 0 and 97. Neither is reachable, for the
/// same reason: everything is normalised into 30–37 first. They are not ported,
/// and a control confirmed nothing observes their absence.
#[must_use]
pub fn standard_color(code: u8) -> Color {
    match code {
        30 => Color::Black,
        31 => Color::Red,
        32 => Color::Green,
        33 => Color::Yellow,
        34 => Color::Blue,
        35 => Color::Magenta,
        36 => Color::Cyan,
        _ => Color::TextPrimary,
    }
}

/// What a `;`-separated list of SGR codes means.
///
/// A later code overwrites an earlier one, so `31;32` is green: the terminal's
/// own rule, and the reason this is a fold rather than a first-match.
///
/// Code 1 — bold, which should brighten the next foreground — is read and
/// deliberately ignored, as the C++ comment says. Dropping it silently is
/// better than the alternative of treating it as an unknown code, because a
/// script emitting `1;31` still gets red.
#[must_use]
pub fn parse_color(codes: &[u8]) -> Format {
    let mut fmt = Format::default();

    for &code in codes {
        match code {
            30..=37 => fmt.foreground = Some(standard_color(code)),
            40..=47 => fmt.background = Some(standard_color(code - 10)),
            0 => fmt.reset = true,
            1 => {}
            90..=97 => fmt.foreground = Some(standard_color(code - 60)),
            _ => {}
        }
    }

    fmt
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Normal,
    Url,
    Escape,
    Color,
}

/// Reads output one run at a time.
///
/// Indexed by `char` where the C++ indexes by UTF-16 code unit. The two agree
/// for everything in the BMP, and neither splits a character the other keeps
/// whole.
#[derive(Debug)]
pub struct Tokenizer {
    data: Vec<char>,
    cursor: usize,
    state: State,
}

impl Tokenizer {
    /// Reads `text` from the beginning.
    #[must_use]
    pub fn new(text: &str) -> Self {
        Self {
            data: text.chars().collect(),
            cursor: 0,
            state: State::Normal,
        }
    }

    /// How far through the text the reader is.
    #[must_use]
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// Moves the reader, for output that arrives a piece at a time.
    pub fn set_cursor(&mut self, cursor: usize) {
        self.cursor = cursor;
    }

    /// The next run, or `None` at the end of the text.
    ///
    /// Three of the four states end a run by returning **without consuming the
    /// character that ended it**, so the next call sees it again in the new
    /// state. That is what lets a link start at the `h` of `http` rather than
    /// one character late, and what lets the character after a link be read as
    /// ordinary text rather than dropped.
    #[allow(clippy::missing_panics_doc)]
    pub fn next_token(&mut self) -> Option<Token> {
        if self.cursor >= self.data.len() {
            return None;
        }

        let mut token = Token::default();
        let mut codes: Vec<u8> = Vec::new();

        while self.cursor < self.data.len() {
            let ch = self.data[self.cursor];

            match self.state {
                State::Normal => {
                    if ch == '\u{1b}' {
                        // A run carries at most one format: a second escape
                        // ends it, so the colour a sequence set applies to the
                        // text after it and nothing else.
                        if token.fmt.is_some() {
                            return Some(token);
                        }
                        self.state = State::Escape;
                    } else if self.starts_url() {
                        self.state = State::Url;
                        return Some(token);
                    } else {
                        token.text.push(ch);
                    }
                }
                State::Url => {
                    if is_valid_url_char(ch) {
                        token.text.push(ch);
                    } else {
                        self.state = State::Normal;
                        token.url = true;
                        return Some(token);
                    }
                }
                State::Escape => {
                    // Everything until a `[` is swallowed, so an escape
                    // sequence that is not a CSI eats the text after it until
                    // one turns up. Reproduced rather than fixed: it is what a
                    // user's script has already been rendered through.
                    if ch == '[' {
                        if !token.text.is_empty() {
                            return Some(token);
                        }
                        codes.clear();
                        codes.push(0);
                        self.state = State::Color;
                    }
                }
                State::Color => {
                    if ch.is_ascii_digit() {
                        let digit = ch as u8 - b'0';
                        let last = codes.last_mut().expect("a code is pushed on entry");
                        // `uint8_t` arithmetic in the C++, wrapping and all: a
                        // long run of digits is nonsense either way, and this
                        // is the nonsense that ships.
                        *last = last.wrapping_mul(10).wrapping_add(digit);
                    } else if ch == ';' {
                        codes.push(0);
                    } else if ch == 'm' {
                        token.fmt = Some(parse_color(&codes));
                        self.state = State::Normal;
                    }
                }
            }

            self.cursor += 1;
        }

        Some(token)
    }

    fn starts_url(&self) -> bool {
        let remaining = &self.data[self.cursor..];

        URL_SCHEMES.iter().any(|scheme| {
            let scheme: Vec<char> = scheme.chars().collect();
            remaining.len() >= scheme.len() && remaining[..scheme.len()] == scheme[..]
        })
    }
}
