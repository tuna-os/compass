//! What a snippet expands to, which is what the machine then types.
//!
//! Read off `SnippetExpander`
//! (`src/server/src/services/snippet/snippet-expander.hpp`).

use std::cell::RefCell;

use compass_core::shortcut::parse_link;
use compass_core::snippet_expander::{
    DEFAULT_DATE_FORMAT, ExpansionContext, Options, ResultPart, SHELL_TIMEOUT_MS, ShellCommand,
    expand, expand_to_string, shell_commands,
};

/// A world with a fixed clipboard, uuid and clock.
#[derive(Default)]
struct World {
    clipboard: String,
    /// Every set of shell commands it was asked to run.
    ran: RefCell<Vec<Vec<ShellCommand>>>,
    /// What each run returns.
    shell_output: Vec<String>,
}

impl ExpansionContext for World {
    fn clipboard_text(&self) -> String {
        self.clipboard.clone()
    }
    fn uuid(&self) -> String {
        "0123-4567".to_owned()
    }
    fn formatted_date(&self, format: &str) -> String {
        format!("<date:{format}>")
    }
    fn run_shell_commands(&self, commands: &[ShellCommand]) -> Vec<String> {
        self.ran.borrow_mut().push(commands.to_vec());
        self.shell_output.clone()
    }
}

/// A world that panics if a shell command is run.
struct NoShell;

impl ExpansionContext for NoShell {
    fn clipboard_text(&self) -> String {
        String::new()
    }
    fn uuid(&self) -> String {
        "0123-4567".to_owned()
    }
    fn formatted_date(&self, format: &str) -> String {
        format!("<date:{format}>")
    }
    fn run_shell_commands(&self, _commands: &[ShellCommand]) -> Vec<String> {
        panic!("no shell command should run here")
    }
}

/// Expand `text` with `arguments`.
fn expand_text(text: &str, arguments: &[(&str, &str)], world: &impl ExpansionContext) -> String {
    let owned: Vec<(String, String)> = arguments
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect();
    expand_to_string(&parse_link(text).parts, &owned, world)
}

#[test]
fn the_shell_timeout_is_two_seconds() {
    assert_eq!(SHELL_TIMEOUT_MS, 2000);
}

#[test]
fn plain_text_expands_to_itself() {
    assert_eq!(expand_text("hello world", &[], &NoShell), "hello world");
}

#[test]
fn a_named_argument_is_substituted() {
    assert_eq!(
        expand_text("Dear {name},", &[("name", "Ada")], &NoShell),
        "Dear Ada,"
    );
}

#[test]
fn the_long_and_short_argument_spellings_mean_the_same_thing() {
    // {argument name=x} is what the editor writes; {x} is what people type.
    let long = expand_text("{argument name=who}", &[("who", "Ada")], &NoShell);
    let short = expand_text("{who}", &[("who", "Ada")], &NoShell);
    assert_eq!(long, "Ada");
    assert_eq!(short, "Ada");
}

#[test]
fn an_argument_with_no_value_expands_to_nothing_at_all() {
    // Not an empty part: the cursor offset counts what will really be typed,
    // so an empty part would be harmless here and wrong the moment a cursor
    // placeholder follows it.
    let expansion = expand(
        &parse_link("a{missing}b").parts,
        &[],
        Options::default(),
        &NoShell,
    );
    assert_eq!(expansion.to_text(), "ab");
    assert_eq!(expansion.parts.len(), 2, "no empty part in between");
}

#[test]
fn an_argument_placeholder_with_no_name_expands_to_nothing() {
    assert_eq!(expand_text("a{argument}b", &[("", "x")], &NoShell), "ab");

    // And specifically does not fall back to its own id: an argument that
    // happens to be called "argument" must not be picked up by a placeholder
    // that named nothing.
    assert_eq!(
        expand_text("a{argument}b", &[("argument", "WRONG")], &NoShell),
        "ab"
    );
}

#[test]
fn the_clipboard_is_substituted() {
    let world = World {
        clipboard: "copied text".to_owned(),
        ..World::default()
    };
    assert_eq!(expand_text("{clipboard}", &[], &world), "copied text");
}

#[test]
fn the_uuid_is_stable_across_expansions() {
    // A preview redrawn on every keystroke would otherwise show a different
    // identifier each time, which reads as the snippet being wrong.
    let world = World::default();
    let first = expand_text("{uuid}", &[], &world);
    let second = expand_text("{uuid}", &[], &world);
    assert_eq!(first, second);
    assert!(!first.is_empty());
}

#[test]
fn the_same_uuid_appears_at_every_use_in_one_snippet() {
    let world = World::default();
    // Separated by something a uuid cannot contain, since a uuid has dashes
    // in it.
    let text = expand_text("{uuid}|{uuid}", &[], &world);
    let (left, right) = text.split_once('|').expect("two uuids");
    assert_eq!(left, right);
    assert!(!left.is_empty());
}

#[test]
fn the_date_uses_the_cpp_default_format() {
    assert_eq!(DEFAULT_DATE_FORMAT, "yyyy-MM-dd hh:mm");
    assert_eq!(
        expand_text("{date}", &[], &NoShell),
        format!("<date:{DEFAULT_DATE_FORMAT}>")
    );
}

#[test]
fn a_date_format_argument_is_honoured() {
    assert_eq!(
        expand_text("{date format=yyyy}", &[], &NoShell),
        "<date:yyyy>"
    );
}

#[test]
fn the_cursor_marks_a_position_and_types_nothing() {
    let expansion = expand(
        &parse_link("ab{cursor}cd").parts,
        &[],
        Options::default(),
        &NoShell,
    );
    assert_eq!(expansion.to_text(), "abcd");
    assert_eq!(expansion.cursor_position, Some(2));
    assert_eq!(
        expansion.parts.len(),
        2,
        "the cursor contributes no part at all, not an empty one: {:?}",
        expansion.parts
    );
}

#[test]
fn the_cursor_counts_what_has_been_substituted_so_far() {
    // Counted over the parts already built, so a placeholder before it shifts
    // the caret by however much it expanded to — not by the length of the
    // placeholder as written.
    let world = World {
        clipboard: "1234567890".to_owned(),
        ..World::default()
    };
    let expansion = expand(
        &parse_link("{clipboard}{cursor}end").parts,
        &[],
        Options::default(),
        &world,
    );
    assert_eq!(expansion.cursor_position, Some(10));
}

#[test]
fn a_snippet_with_no_cursor_has_no_position() {
    let expansion = expand(
        &parse_link("plain").parts,
        &[],
        Options::default(),
        &NoShell,
    );
    assert_eq!(expansion.cursor_position, None);
}

#[test]
fn the_last_cursor_wins() {
    // Each one overwrites the position, and the caret can only be in one place.
    let expansion = expand(
        &parse_link("a{cursor}bb{cursor}c").parts,
        &[],
        Options::default(),
        &NoShell,
    );
    assert_eq!(expansion.cursor_position, Some(3));
}

#[test]
fn the_cursor_position_counts_characters_not_bytes() {
    // A snippet with an accented word before the cursor would otherwise put
    // the caret past where the person can see it.
    let expansion = expand(
        &parse_link("café{cursor}").parts,
        &[],
        Options::default(),
        &NoShell,
    );
    assert_eq!(expansion.cursor_position, Some(4));
}

#[test]
fn a_shell_placeholder_runs_and_its_output_is_substituted() {
    let world = World {
        shell_output: vec!["output".to_owned()],
        ..World::default()
    };
    assert_eq!(expand_text("{shell code=echo}", &[], &world), "output");
    assert_eq!(world.ran.borrow().len(), 1);
    assert_eq!(world.ran.borrow()[0][0].code, "echo");
}

#[test]
fn a_preview_does_not_run_anything() {
    // Running a command because a snippet scrolled past in a list would be a
    // side effect nobody asked for, and a slow one blocks the list.
    let expansion = expand(
        &parse_link(r#"{shell code="rm -rf /tmp/x"}"#).parts,
        &[],
        Options {
            execute_shell: false,
        },
        &NoShell,
    );
    assert_eq!(expansion.to_text(), "$(rm -rf /tmp/x)");
}

#[test]
fn a_shell_command_with_spaces_has_to_be_quoted() {
    // The placeholder parser ends an unquoted value at the first space, and
    // then loses the whole placeholder rather than keeping a truncated one.
    // Worth knowing: a snippet author who forgets the quotes gets silence, not
    // a truncated command, so nothing dangerous is half-run.
    let unquoted = parse_link("{shell code=rm -rf /tmp/x}");
    assert!(
        !unquoted
            .parts
            .iter()
            .any(|part| matches!(part, compass_core::shortcut::UrlPart::Placeholder(_))),
        "the placeholder is dropped entirely: {:?}",
        unquoted.parts
    );

    let quoted = parse_link(r#"{shell code="rm -rf /tmp/x"}"#);
    let commands = shell_commands(&quoted.parts);
    assert_eq!(commands.len(), 1);
    assert_eq!(commands[0].code, "rm -rf /tmp/x");
}

#[test]
fn an_unexecuted_shell_placeholder_reads_like_the_command() {
    // "$(...)" rather than a gap, so a preview says what will happen.
    let expansion = expand(
        &parse_link("before {shell code=date} after").parts,
        &[],
        Options {
            execute_shell: false,
        },
        &NoShell,
    );
    assert_eq!(expansion.to_text(), "before $(date) after");
}

#[test]
fn every_shell_placeholder_is_collected_before_any_of_them_runs() {
    // The C++ starts them concurrently and then waits, so three placeholders
    // take as long as the slowest rather than the sum.
    let world = World {
        shell_output: vec!["a".to_owned(), "b".to_owned()],
        ..World::default()
    };
    expand_text("{shell code=one}{shell code=two}", &[], &world);

    assert_eq!(world.ran.borrow().len(), 1, "one batch, not one call each");
    assert_eq!(world.ran.borrow()[0].len(), 2);
}

#[test]
fn shell_outputs_are_matched_to_placeholders_in_order() {
    let world = World {
        shell_output: vec!["first".to_owned(), "second".to_owned()],
        ..World::default()
    };
    assert_eq!(
        expand_text("{shell code=a}-{shell code=b}", &[], &world),
        "first-second"
    );
}

#[test]
fn a_run_returning_too_few_results_does_not_shift_the_later_ones() {
    // The index advances on both paths, so a failed second command shows its
    // own code rather than the third command's output.
    let world = World {
        shell_output: vec!["first".to_owned()],
        ..World::default()
    };
    assert_eq!(
        expand_text("{shell code=a}|{shell code=b}", &[], &world),
        "first|$(b)"
    );
}

#[test]
fn a_shell_placeholder_can_name_its_interpreter() {
    let world = World {
        shell_output: vec!["out".to_owned()],
        ..World::default()
    };
    expand_text("{shell code=print exec=python3}", &[], &world);

    let ran = world.ran.borrow();
    assert_eq!(ran[0][0].exec, "python3");
    assert_eq!(ran[0][0].code, "print");
}

#[test]
fn a_shell_placeholder_with_no_interpreter_leaves_it_empty() {
    // Empty means the person's login shell, which is what the app service
    // supplies; a default hardcoded here would ignore their choice.
    let commands = shell_commands(&parse_link("{shell code=ls}").parts);
    assert_eq!(commands.len(), 1);
    assert_eq!(commands[0].exec, "");
}

#[test]
fn a_snippet_with_no_shell_placeholders_runs_nothing() {
    let world = World::default();
    expand_text("plain {name}", &[("name", "x")], &world);
    assert!(world.ran.borrow().is_empty());
}

#[test]
fn substituted_placeholders_are_marked_and_arguments_are_not() {
    // The C++ pushes an argument's value with the default placeholder=false,
    // unlike every other substitution. Whatever highlights placeholders in a
    // preview therefore leaves argument values looking like ordinary text.
    let world = World {
        clipboard: "clip".to_owned(),
        ..World::default()
    };
    let arguments = vec![("who".to_owned(), "Ada".to_owned())];
    let expansion = expand(
        &parse_link("{who}{clipboard}{uuid}").parts,
        &arguments,
        Options::default(),
        &world,
    );

    let marked: Vec<bool> = expansion
        .parts
        .iter()
        .filter(|part| !part.text.is_empty())
        .map(|part| part.placeholder)
        .collect();
    assert_eq!(marked, vec![false, true, true]);
}

#[test]
fn snippet_text_is_never_marked_as_a_placeholder() {
    let expansion = expand(
        &parse_link("plain").parts,
        &[],
        Options::default(),
        &NoShell,
    );
    assert!(expansion.parts.iter().all(|part| !part.placeholder));
    assert!(!ResultPart::literal("x").placeholder);
    assert!(ResultPart::substituted("x").placeholder);
}

#[test]
fn an_unknown_placeholder_expands_to_nothing_rather_than_failing() {
    // A snippet written against a newer build still types most of itself.
    assert_eq!(expand_text("a{somethingnew}b", &[], &NoShell), "ab");
}

#[test]
fn everything_combines() {
    let world = World {
        clipboard: "CLIP".to_owned(),
        shell_output: vec!["SHELL".to_owned()],
        ..World::default()
    };
    let arguments = vec![("who".to_owned(), "Ada".to_owned())];
    let expansion = expand(
        &parse_link("Hi {who}: {clipboard} {shell code=x}{cursor}!").parts,
        &arguments,
        Options::default(),
        &world,
    );
    assert_eq!(expansion.to_text(), "Hi Ada: CLIP SHELL!");
    assert_eq!(
        expansion.cursor_position,
        Some("Hi Ada: CLIP SHELL".chars().count())
    );
}
