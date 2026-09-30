//! A snippet's text and its `{placeholders}`: `PlaceholderString::parse`.
//!
//! A port of `src/server/src/utils/placeholder.cpp`. It is the quicklink
//! parser ([`crate::shortcut::parse_link`]) with one more state: a backslash
//! escapes the character after it, so `\{` is a literal brace rather than the
//! start of a placeholder and `\\` is one backslash. Another escaped
//! character is kept and the backslash dropped; a backslash that ends the text
//! is kept. Everything else — the id, `key=value` arguments, quoted values,
//! a placeholder cut off by the end of the text — reads as a quicklink does.
//!
//! The parts are [`crate::shortcut::UrlPart`]s, so the snippet expander takes
//! either parser's output.

use crate::shortcut::{Argument, Link, Placeholder, UrlPart};

/// The placeholders a snippet reserves (`parseSnippetText`); any other id is
/// an argument named after itself.
pub const SNIPPET_RESERVED_IDS: &[&str] = &["uuid", "clipboard", "date", "cursor", "shell"];

#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    Normal,
    Escape,
    Id,
    KeyStart,
    Key,
    ValueStart,
    Value,
    ValueQuoted,
}

/// Parses a snippet's text, as `PlaceholderString::parseSnippetText` does.
#[must_use]
pub fn parse_snippet_text(text: &str) -> Link {
    parse(text, SNIPPET_RESERVED_IDS)
}

/// Parses `text` with `reserved` as the ids that are not arguments, as
/// `PlaceholderString::parse` does.
#[must_use]
#[allow(clippy::too_many_lines)]
pub fn parse(text: &str, reserved: &[&str]) -> Link {
    let chars: Vec<char> = text.chars().collect();
    let slice = |from: usize, to: usize| -> String { chars[from..to].iter().collect() };

    let mut parsed = Link {
        raw: text.to_owned(),
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
                if ch == '\\' {
                    parsed.parts.push(UrlPart::Text(slice(start, i)));
                    state = State::Escape;
                } else if ch == '{' {
                    parsed.parts.push(UrlPart::Text(slice(start, i)));
                    state = State::Id;
                    start = i + 1;
                }
            }
            State::Escape => {
                // `\\` is one backslash, as a part of its own; any other
                // character starts the next run of text, read as text.
                if ch == '\\' {
                    parsed.parts.push(UrlPart::Text("\\".to_owned()));
                    start = i + 1;
                } else {
                    start = i;
                }
                state = State::Normal;
            }
            State::Id => {
                if !ch.is_alphanumeric() {
                    placeholder.id = slice(start, i);
                    start = i;
                    state = State::KeyStart;
                    continue;
                }
            }
            State::KeyStart => {
                if ch == '}' {
                    parsed.parts.push(UrlPart::Placeholder(placeholder.clone()));
                    insert_placeholder(&mut parsed, &placeholder, reserved);
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

    if state == State::Normal && chars.len() > start {
        parsed.parts.push(UrlPart::Text(slice(start, chars.len())));
    } else if state == State::Escape {
        parsed.parts.push(UrlPart::Text("\\".to_owned()));
    }

    parsed
}

/// `insertPlaceholder`: `argument` names its argument with `name=` and
/// `default=`; any other id that is not reserved is an argument by its id;
/// an argument is listed once, by name.
fn insert_placeholder(parsed: &mut Link, placeholder: &Placeholder, reserved: &[&str]) {
    let is_reserved = reserved.contains(&placeholder.id.as_str());
    if !is_reserved || placeholder.id == "argument" {
        let argument = if placeholder.id == "argument" {
            Argument {
                name: placeholder.args.get("name").cloned().unwrap_or_default(),
                default_value: placeholder.args.get("default").cloned().unwrap_or_default(),
            }
        } else {
            Argument {
                name: placeholder.id.clone(),
                default_value: String::new(),
            }
        };
        if !parsed
            .arguments
            .iter()
            .any(|known| known.name == argument.name)
        {
            parsed.arguments.push(argument);
        }
    }
    parsed.placeholders.push(placeholder.clone());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(parts: &[UrlPart]) -> String {
        parts
            .iter()
            .map(|part| match part {
                UrlPart::Text(text) => text.clone(),
                UrlPart::Placeholder(placeholder) => format!("<{}>", placeholder.id),
            })
            .collect()
    }

    #[test]
    fn an_escaped_brace_is_text_and_not_a_placeholder() {
        let parsed = parse_snippet_text(r"fn main() \{ {name} }");
        assert_eq!(text(&parsed.parts), "fn main() { <name> }");
        assert_eq!(parsed.placeholders.len(), 1);
        assert_eq!(parsed.arguments[0].name, "name");
    }

    #[test]
    fn a_doubled_backslash_is_one_and_the_brace_after_it_opens_a_placeholder() {
        let parsed = parse_snippet_text(r"C:\\{clipboard}");
        assert_eq!(text(&parsed.parts), r"C:\<clipboard>");
        assert!(parsed.arguments.is_empty(), "clipboard is reserved");
    }

    #[test]
    fn another_escaped_character_loses_its_backslash_and_a_trailing_one_stays() {
        assert_eq!(text(&parse_snippet_text(r"a\nb").parts), "anb");
        assert_eq!(text(&parse_snippet_text("end\\").parts), "end\\");
    }

    #[test]
    fn without_a_backslash_it_reads_as_a_quicklink_does() {
        let raw =
            "Hi {name}, {argument name=\"topic\" default=\"news\"} {cursor}{date format=yyyy}";
        assert_eq!(
            parse_snippet_text(raw).parts,
            crate::shortcut::parse_link(raw).parts
        );
        let arguments = parse_snippet_text(raw).arguments;
        assert_eq!(
            arguments
                .iter()
                .map(|a| (a.name.as_str(), a.default_value.as_str()))
                .collect::<Vec<_>>(),
            [("name", ""), ("topic", "news")]
        );
    }
}
