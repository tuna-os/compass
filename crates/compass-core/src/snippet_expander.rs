//! Turning a snippet's text into what actually gets typed.
//!
//! A port of `SnippetExpander`
//! (`src/server/src/services/snippet/snippet-expander.hpp`), reusing the
//! placeholder parser already in [`crate::shortcut`].
//!
//! # What a snippet expands to is what a person's machine then types
//!
//! Every rule here decides characters that are about to be injected into
//! whatever window has focus. An argument that silently expands to nothing is
//! a half-written email; a cursor position computed from the wrong parts puts
//! the caret in the middle of a word; a shell placeholder that runs when it
//! should not is a command executed because somebody scrolled past a snippet
//! in a list. So the rules are small and each has a test naming the thing it
//! prevents.

use crate::shortcut::{Placeholder, UrlPart};

/// The default `date` format, from the C++ literal.
pub const DEFAULT_DATE_FORMAT: &str = "yyyy-MM-dd hh:mm";

/// How long a shell placeholder may run, from `SHELL_TIMEOUT_MS`.
pub const SHELL_TIMEOUT_MS: u64 = 2000;

/// The placeholder that marks where the caret should end up.
pub const CURSOR_ID: &str = "cursor";
/// The placeholder replaced by the clipboard's text.
pub const CLIPBOARD_ID: &str = "clipboard";
/// The placeholder replaced by a stable identifier.
pub const UUID_ID: &str = "uuid";
/// The placeholder replaced by the current date.
pub const DATE_ID: &str = "date";
/// The placeholder replaced by a command's output.
pub const SHELL_ID: &str = "shell";
/// The placeholder replaced by a named argument.
pub const ARGUMENT_ID: &str = "argument";

/// One piece of an expansion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResultPart {
    /// The text.
    pub text: String,
    /// Whether it came from a placeholder rather than the snippet's own text.
    ///
    /// # Arguments are *not* marked
    ///
    /// The C++ pushes an argument's value with the default `placeholder =
    /// false`, unlike every other substitution. Whatever highlights
    /// placeholders in a preview therefore leaves argument values looking like
    /// ordinary text — which is arguably right, since the person typed them —
    /// and this port keeps it rather than making the flag mean something
    /// slightly different from what the preview was built against.
    pub placeholder: bool,
}

impl ResultPart {
    /// A piece of the snippet's own text.
    #[must_use]
    pub fn literal(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            placeholder: false,
        }
    }

    /// A substituted placeholder.
    #[must_use]
    pub fn substituted(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            placeholder: true,
        }
    }
}

/// An expanded snippet.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Expansion {
    /// The pieces, in order.
    pub parts: Vec<ResultPart>,
    /// Where the caret should go, in characters from the start.
    pub cursor_position: Option<usize>,
}

impl Expansion {
    /// The whole expansion as one string.
    #[must_use]
    pub fn to_text(&self) -> String {
        self.parts.iter().map(|part| part.text.as_str()).collect()
    }
}

/// What a `shell` placeholder asks to run.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ShellCommand {
    /// The code to run.
    pub code: String,
    /// The interpreter, or empty for the person's login shell.
    pub exec: String,
}

/// How to expand.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Options {
    /// Whether `shell` placeholders actually run.
    ///
    /// Off for previews, which are recomputed on every keystroke as somebody
    /// searches: running a command because a snippet scrolled past would be a
    /// side effect nobody asked for, and a slow one blocks the list.
    pub execute_shell: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            execute_shell: true,
        }
    }
}

/// Everything the expander needs from outside itself.
pub trait ExpansionContext {
    /// The clipboard's current text.
    fn clipboard_text(&self) -> String;

    /// A stable identifier for this expander.
    ///
    /// Generated once and reused, as the C++ member is: a preview redrawn on
    /// every keystroke would otherwise show a different uuid each time.
    fn uuid(&self) -> String;

    /// The current date in `format`.
    fn formatted_date(&self, format: &str) -> String;

    /// Run every shell placeholder, in order, returning each one's output.
    ///
    /// All of them at once because the C++ starts them concurrently and then
    /// waits: three placeholders take as long as the slowest, not the sum.
    fn run_shell_commands(&self, commands: &[ShellCommand]) -> Vec<String>;
}

/// The `shell` commands in `parts`, in order.
#[must_use]
pub fn shell_commands(parts: &[UrlPart]) -> Vec<ShellCommand> {
    parts
        .iter()
        .filter_map(|part| match part {
            UrlPart::Placeholder(placeholder) if placeholder.id == SHELL_ID => Some(ShellCommand {
                code: placeholder.args.get("code").cloned().unwrap_or_default(),
                exec: placeholder.args.get("exec").cloned().unwrap_or_default(),
            }),
            _ => None,
        })
        .collect()
}

/// The text a non-executed `shell` placeholder shows.
///
/// `$(code)`, so a preview reads like the shell command it will run rather
/// than like a gap.
#[must_use]
pub fn unexecuted_shell_text(placeholder: &Placeholder) -> String {
    let code = placeholder.args.get("code").cloned().unwrap_or_default();
    format!("$({code})")
}

/// The value of the argument a placeholder names, if it was given one.
///
/// `{argument name=x}` and a bare `{x}` mean the same thing; the first is what
/// the editor writes and the second is what people type.
fn argument_value(placeholder: &Placeholder, arguments: &[(String, String)]) -> Option<String> {
    let name = if placeholder.id == ARGUMENT_ID {
        placeholder.args.get("name")?
    } else {
        &placeholder.id
    };
    arguments
        .iter()
        .find(|(candidate, _)| candidate == name)
        .map(|(_, value)| value.clone())
}

/// Expand `parts` with `arguments`.
///
/// # Errors
///
/// None: an unknown placeholder expands to nothing rather than failing, so a
/// snippet written against a newer build still types most of itself.
#[must_use]
pub fn expand(
    parts: &[UrlPart],
    arguments: &[(String, String)],
    options: Options,
    context: &impl ExpansionContext,
) -> Expansion {
    let commands = shell_commands(parts);
    let shell_results = if options.execute_shell && !commands.is_empty() {
        context.run_shell_commands(&commands)
    } else {
        Vec::new()
    };

    let mut result = Expansion::default();
    let mut shell_index = 0usize;

    for part in parts {
        let placeholder = match part {
            UrlPart::Text(text) => {
                result.parts.push(ResultPart::literal(text.clone()));
                continue;
            }
            UrlPart::Placeholder(placeholder) => placeholder,
        };

        match placeholder.id.as_str() {
            CURSOR_ID => {
                // Counted over the parts *so far*, so two cursors leave the
                // caret at the last one — and the placeholder itself
                // contributes nothing, which is what makes the count right.
                result.cursor_position = Some(
                    result
                        .parts
                        .iter()
                        .map(|part| part.text.chars().count())
                        .sum(),
                );
            }
            CLIPBOARD_ID => result
                .parts
                .push(ResultPart::substituted(context.clipboard_text())),
            UUID_ID => result.parts.push(ResultPart::substituted(context.uuid())),
            DATE_ID => {
                let format = placeholder
                    .args
                    .get("format")
                    .map_or(DEFAULT_DATE_FORMAT, String::as_str);
                result
                    .parts
                    .push(ResultPart::substituted(context.formatted_date(format)));
            }
            SHELL_ID => {
                let text = shell_results
                    .get(shell_index)
                    .cloned()
                    .unwrap_or_else(|| unexecuted_shell_text(placeholder));
                result.parts.push(ResultPart::substituted(text));
                // Incremented on both paths, so a run that returned fewer
                // results than there were placeholders does not shift every
                // later one onto the wrong command.
                shell_index += 1;
            }
            _ => {
                if let Some(value) = argument_value(placeholder, arguments) {
                    result.parts.push(ResultPart::literal(value));
                }
                // An argument with no value emits nothing at all -- not an
                // empty part -- so the cursor offset counts what will really
                // be typed.
            }
        }
    }

    result
}

/// Expand and join, as `expandToString` does.
#[must_use]
pub fn expand_to_string(
    parts: &[UrlPart],
    arguments: &[(String, String)],
    context: &impl ExpansionContext,
) -> String {
    expand(parts, arguments, Options::default(), context).to_text()
}

/// The placeholders a snippet's text reserves, as `parseSnippetText` lists
/// them; every other placeholder, and `argument`, is an argument.
pub const RESERVED_IDS: &[&str] = &[UUID_ID, CLIPBOARD_ID, DATE_ID, CURSOR_ID, SHELL_ID];

/// The arguments a snippet's text asks for, in order of first appearance and
/// each name once: an argument is identified by its name and can be expanded
/// in several places, as `PlaceholderString::parse` collects them.
///
/// `{argument}` with no `name=` names nothing the expander could fill, and is
/// left out rather than asked for under an empty label.
#[must_use]
pub fn arguments(parts: &[UrlPart]) -> Vec<crate::shortcut::Argument> {
    let mut found: Vec<crate::shortcut::Argument> = Vec::new();
    for part in parts {
        let UrlPart::Placeholder(placeholder) = part else {
            continue;
        };
        let argument = if placeholder.id == ARGUMENT_ID {
            let Some(name) = placeholder.args.get("name") else {
                continue;
            };
            crate::shortcut::Argument {
                name: name.clone(),
                default_value: placeholder.args.get("default").cloned().unwrap_or_default(),
            }
        } else if RESERVED_IDS.contains(&placeholder.id.as_str()) {
            continue;
        } else {
            crate::shortcut::Argument {
                name: placeholder.id.clone(),
                default_value: String::new(),
            }
        };
        if !found.iter().any(|known| known.name == argument.name) {
            found.push(argument);
        }
    }
    found
}

#[cfg(test)]
mod argument_tests {
    use super::*;

    #[test]
    fn arguments_are_named_once_and_reserved_ids_are_not_arguments() {
        let parts = crate::shortcut::parse_link(
            "Hi {name}, {argument name=\"topic\" default=\"news\"} {cursor}{clipboard} \
             {name} {argument} {date format=yyyy}",
        )
        .parts;
        let found = arguments(&parts);
        assert_eq!(
            found
                .iter()
                .map(|a| (a.name.as_str(), a.default_value.as_str()))
                .collect::<Vec<_>>(),
            [("name", ""), ("topic", "news")]
        );
    }
}
