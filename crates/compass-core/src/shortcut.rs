//! Quicklinks: a URL with `{placeholders}` in it, and what they parse to.
//!
//! A port of `Shortcut::parseLink` and `Shortcut::insertPlaceholder`
//! (`src/server/src/services/shortcut/shortcut.cpp`). The parser is a
//! hand-written state machine over the raw link, and every one of its edges is
//! observable from the outside: a malformed link does not fail, it produces
//! whatever the machine was holding when the string ran out.
//!
//! The link is split into [`UrlPart`]s — literal text and placeholders — so the
//! UI can show the shape of a quicklink, and placeholders that are not reserved
//! become [`Argument`]s the user is asked to fill in.

use std::collections::BTreeMap;

/// Placeholder ids with expansion rules of their own.
///
/// Verbatim from `m_reservedPlaceholderIds`. Anything else is an argument
/// named after the id, which is what lets `?q={query}` stand in for the longer
/// `?q={argument name="query"}`.
pub const RESERVED_PLACEHOLDER_IDS: &[&str] =
    &["clipboard", "selection", "selected", "uuid", "date"];

/// One `{id key=value}` in a link.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Placeholder {
    /// The word right after the brace.
    pub id: String,
    /// Its `key=value` arguments.
    ///
    /// The C++ collects these into a `std::map` with `insert`, which **keeps
    /// the first** value for a repeated key rather than overwriting it. A
    /// `BTreeMap` with the same "first wins" rule reproduces that, and is
    /// ordered for the same reason `std::map` is.
    pub args: BTreeMap<String, String>,
}

/// A piece of a parsed link.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UrlPart {
    /// Literal text.
    Text(String),
    /// A placeholder to expand.
    Placeholder(Placeholder),
}

/// Something the user is asked for before the link opens.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Argument {
    /// What to call the field.
    pub name: String,
    /// What to prefill it with.
    pub default_value: String,
}

/// A parsed link.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Link {
    /// The link as it was given.
    pub raw: String,
    /// Text and placeholders, in order.
    pub parts: Vec<UrlPart>,
    /// Every placeholder, reserved or not, in order.
    pub placeholders: Vec<Placeholder>,
    /// The placeholders that are arguments, in order.
    pub arguments: Vec<Argument>,
}

/// The default value of `Shortcut::app`, meaning "whatever opens this".
pub const DEFAULT_APP_ID: &str = "default";

#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    Normal,
    Id,
    KeyStart,
    Key,
    ValueStart,
    Value,
    ValueQuoted,
}

/// Parses `link`, as `Shortcut::parseLink` does.
///
/// Qt indexes a `QString` by UTF-16 code unit and this indexes by `char`;
/// the two agree for everything in the BMP, which is every link anyone has
/// written, and neither slices inside a character the other would keep whole.
#[must_use]
#[allow(clippy::too_many_lines)]
pub fn parse_link(link: &str) -> Link {
    let chars: Vec<char> = link.chars().collect();
    let slice = |from: usize, to: usize| -> String { chars[from..to].iter().collect() };

    let mut parsed = Link {
        raw: link.to_owned(),
        ..Link::default()
    };
    let mut state = State::Normal;
    let mut start = 0usize;
    let mut placeholder = Placeholder::default();
    let mut key = String::new();
    let mut value = String::new();
    let mut i = 0usize;

    while i < chars.len() {
        let ch = chars[i];

        match state {
            State::Normal => {
                if ch == '{' {
                    parsed.parts.push(UrlPart::Text(slice(start, i)));
                    state = State::Id;
                    start = i + 1;
                }
            }
            State::Id => {
                // The id ends at the first character that is not a letter or a
                // number -- a space, an `=`, or the closing brace.
                if !ch.is_alphanumeric() {
                    placeholder.id = slice(start, i);
                    start = i;
                    state = State::KeyStart;
                    // `startPos = i--` then `++i`: the same character is read
                    // again in the next state.
                    continue;
                }
            }
            State::KeyStart => {
                if ch == '}' {
                    parsed.parts.push(UrlPart::Placeholder(placeholder.clone()));
                    insert_placeholder(&mut parsed, &placeholder);
                    placeholder = Placeholder::default();
                    start = i + 1;
                    state = State::Normal;
                } else if !ch.is_whitespace() {
                    start = i;
                    key.clear();
                    value.clear();
                    state = State::Key;
                    continue;
                }
            }
            State::Key => {
                if ch == '=' {
                    key = slice(start, i);
                    state = State::ValueStart;
                }
            }
            State::ValueStart => {
                if !ch.is_whitespace() {
                    start = i;
                    state = State::Value;
                    continue;
                }
            }
            State::Value => {
                if ch == '"' {
                    value.push_str(&slice(start, i));
                    start = i + 1;
                    state = State::ValueQuoted;
                } else if !ch.is_alphanumeric() {
                    value.push_str(&slice(start, i));
                    // `map::insert` keeps the first value for a repeated key.
                    placeholder
                        .args
                        .entry(std::mem::take(&mut key))
                        .or_insert_with(|| std::mem::take(&mut value));
                    key.clear();
                    value.clear();
                    state = State::KeyStart;
                    continue;
                }
            }
            State::ValueQuoted => {
                if ch == '"' {
                    value.push_str(&slice(start, i));
                    start = i + 1;
                    state = State::Value;
                }
            }
        }

        i += 1;
    }

    // Only trailing *text* survives the end of the string: a link that ends
    // inside a placeholder loses it, and everything after the opening brace
    // with it. The C++ does the same, and a quicklink is edited until it looks
    // right, so the loss is visible rather than silent.
    if state == State::Normal && chars.len() > start {
        parsed.parts.push(UrlPart::Text(slice(start, chars.len())));
    }

    parsed
}

/// `Shortcut::insertPlaceholder`.
///
/// A placeholder whose id is not reserved becomes an argument named after the
/// id, unless `name=` says otherwise. (The C++ also tests `id == "argument"`,
/// which can never add anything: `argument` is not in the reserved list, so it
/// has already passed the first test.)
fn insert_placeholder(parsed: &mut Link, placeholder: &Placeholder) {
    let reserved = RESERVED_PLACEHOLDER_IDS.contains(&placeholder.id.as_str());

    if !reserved {
        let mut argument = Argument {
            name: placeholder.id.clone(),
            default_value: String::new(),
        };
        if let Some(name) = placeholder.args.get("name") {
            argument.name.clone_from(name);
        }
        if let Some(default) = placeholder.args.get("default") {
            argument.default_value.clone_from(default);
        }
        parsed.arguments.push(argument);
    }

    parsed.placeholders.push(placeholder.clone());
}
