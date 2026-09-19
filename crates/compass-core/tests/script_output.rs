//! Turning a script's output into styled text.
//!
//! Ported from `ScriptOutputTokenizer`.

use compass_core::script_output::{
    Color, Format, Token, Tokenizer, is_valid_url_char, parse_color, standard_color,
};

fn tokens(text: &str) -> Vec<Token> {
    let mut tokenizer = Tokenizer::new(text);
    let mut found = Vec::new();
    while let Some(token) = tokenizer.next_token() {
        found.push(token);
    }
    found
}

fn texts(text: &str) -> Vec<String> {
    tokens(text).into_iter().map(|token| token.text).collect()
}

#[test]
fn plain_output_is_one_run() {
    assert_eq!(texts("hello world"), ["hello world"]);
}

#[test]
fn empty_output_has_no_runs() {
    assert!(tokens("").is_empty());
}

#[test]
fn a_link_is_its_own_run() {
    let found = tokens("see https://example.test/ ok");

    assert_eq!(found[0].text, "see ");
    assert!(!found[0].url);
    assert_eq!(found[1].text, "https://example.test/");
    assert!(found[1].url);
    assert_eq!(found[2].text, " ok");
}

#[test]
fn a_link_starts_at_its_scheme_rather_than_one_character_late() {
    // The run is returned *without* consuming the `h`, so the next call reads
    // it again in the link state.
    let found = tokens("https://example.test/ ");

    assert_eq!(found[1].text, "https://example.test/");
}

#[test]
fn the_character_that_ended_a_link_is_not_lost() {
    let found = tokens("https://a.test x");

    assert_eq!(found[1].text, "https://a.test");
    assert_eq!(found[2].text, " x");
}

#[test]
fn plain_http_is_a_link_too() {
    let found = tokens("http://a.test ");

    assert!(found[1].url);
}

#[test]
fn another_scheme_is_not_a_link() {
    // A launcher makes these clickable, so anything but http and https is
    // text.
    let found = tokens("ftp://a.test ");

    assert_eq!(found.len(), 1);
    assert!(!found[0].url);
}

#[test]
fn a_link_ends_at_a_closing_bracket() {
    // Printed inside brackets, as most prose prints one. Swallowing the
    // bracket produces a link that 404s.
    let found = tokens("(https://a.test)");

    assert_eq!(found[1].text, "https://a.test");
    assert!(found[1].url);
}

#[test]
fn a_link_ends_at_a_quote() {
    let found = tokens("\"https://a.test\"");

    assert_eq!(found[1].text, "https://a.test");
}

#[test]
fn a_link_that_runs_to_the_end_of_the_output_is_still_a_link_run() {
    let found = tokens("https://a.test");

    // Nothing terminated it, so the loop falls out of the end -- and the C++
    // returns the accumulated token without setting `url`. Pinned because it
    // is visible: the last link in a stream is not clickable until more output
    // arrives after it.
    assert_eq!(found[1].text, "https://a.test");
    assert!(!found[1].url);
}

#[test]
fn a_url_character_is_printable_and_unquoted() {
    assert!(is_valid_url_char('a'));
    assert!(is_valid_url_char('/'));
    assert!(!is_valid_url_char(' '));
    assert!(!is_valid_url_char('\n'));
    assert!(!is_valid_url_char('"'));
    assert!(!is_valid_url_char('\''));
    assert!(!is_valid_url_char('('));
    assert!(!is_valid_url_char(')'));
}

#[test]
fn a_colour_sequence_sets_a_format() {
    let found = tokens("\u{1b}[31mred");

    assert_eq!(found[0].fmt.expect("a format").foreground, Some(Color::Red));
    assert_eq!(found[0].text, "red");
}

#[test]
fn text_before_a_sequence_is_its_own_run() {
    let found = tokens("plain\u{1b}[31mred");

    assert_eq!(found[0].text, "plain");
    assert!(found[0].fmt.is_none());
    assert_eq!(found[1].text, "red");
    assert_eq!(found[1].fmt.expect("a format").foreground, Some(Color::Red));
}

#[test]
fn a_run_carries_at_most_one_format() {
    // A second escape ends the run, so a colour applies to the text after it
    // and nothing else.
    let found = tokens("\u{1b}[31mred\u{1b}[32mgreen");

    assert_eq!(found[0].text, "red");
    assert_eq!(found[1].text, "green");
    assert_eq!(
        found[1].fmt.expect("a format").foreground,
        Some(Color::Green)
    );
}

#[test]
fn two_sequences_with_nothing_between_them_still_make_two_runs() {
    // With text between them the run is ended by the text flush instead, so
    // this is the only input that shows the format check doing anything -- a
    // control was silent until it existed.
    let found = tokens("\u{1b}[31m\u{1b}[32mgreen");

    assert_eq!(found[0].text, "");
    assert_eq!(found[0].fmt.expect("a format").foreground, Some(Color::Red));
    assert_eq!(found[1].text, "green");
    assert_eq!(
        found[1].fmt.expect("a format").foreground,
        Some(Color::Green)
    );
}

#[test]
fn a_semicolon_separates_codes() {
    let found = tokens("\u{1b}[31;42mboth");
    let fmt = found[0].fmt.expect("a format");

    assert_eq!(fmt.foreground, Some(Color::Red));
    assert_eq!(fmt.background, Some(Color::Green));
}

#[test]
fn a_later_code_overwrites_an_earlier_one() {
    let found = tokens("\u{1b}[31;32mgreen");

    assert_eq!(
        found[0].fmt.expect("a format").foreground,
        Some(Color::Green)
    );
}

#[test]
fn a_reset_is_recorded() {
    let found = tokens("\u{1b}[0mplain");

    assert!(found[0].fmt.expect("a format").reset);
}

#[test]
fn a_bare_sequence_carries_the_reset_code_it_starts_with() {
    // The code list starts with a 0, so `ESC[m` reads as a reset.
    let found = tokens("\u{1b}[mplain");

    assert!(found[0].fmt.expect("a format").reset);
}

#[test]
fn bold_is_read_and_ignored() {
    // It should brighten the next colour; the C++ says so and does not. A
    // script emitting `1;31` still gets red, which is the part that matters.
    let found = tokens("\u{1b}[1;31mred");
    let fmt = found[0].fmt.expect("a format");

    assert_eq!(fmt.foreground, Some(Color::Red));
    assert!(!fmt.reset);
}

#[test]
fn an_escape_that_is_not_a_sequence_swallows_what_follows_it() {
    // Reproduced rather than fixed: everything until a `[` is eaten. It is
    // what a user's script has already been rendered through.
    let found = tokens("\u{1b}Xlost\u{1b}[31mred");

    assert_eq!(found[0].text, "red");
}

#[test]
fn digits_before_the_bracket_are_swallowed_rather_than_read_as_codes() {
    // `ESC 3 1 m [ 3 2 m` : everything up to the `[` is eaten, so the codes
    // are the ones *after* it. Read the other way round the `m` would close a
    // sequence early and the `[32m` would be printed as text. Most malformed
    // escapes cannot tell the two readings apart -- this one can.
    let found = tokens("\u{1b}31m[32mgreen");

    assert_eq!(found[0].text, "green");
    assert_eq!(
        found[0].fmt.expect("a format").foreground,
        Some(Color::Green)
    );
}

#[test]
fn a_background_code_uses_the_foreground_table_shifted_down() {
    let found = tokens("\u{1b}[44mblue");

    assert_eq!(
        found[0].fmt.expect("a format").background,
        Some(Color::Blue)
    );
}

#[test]
fn a_bright_code_is_the_plain_colour() {
    // 90-97 are shifted down by sixty, so bright red is red. The palette is
    // the theme's and has no brighter version to reach for.
    let found = tokens("\u{1b}[91mred");

    assert_eq!(found[0].fmt.expect("a format").foreground, Some(Color::Red));
}

#[test]
fn white_draws_in_the_themes_ordinary_foreground() {
    // 37 is absent from the table on purpose. A script colouring its output
    // white would otherwise be invisible on a light theme.
    assert_eq!(standard_color(37), Color::TextPrimary);
    assert_eq!(parse_color(&[37]).foreground, Some(Color::TextPrimary));
    assert_eq!(parse_color(&[97]).foreground, Some(Color::TextPrimary));
}

#[test]
fn every_named_colour_maps_where_it_says() {
    assert_eq!(standard_color(30), Color::Black);
    assert_eq!(standard_color(31), Color::Red);
    assert_eq!(standard_color(32), Color::Green);
    assert_eq!(standard_color(33), Color::Yellow);
    assert_eq!(standard_color(34), Color::Blue);
    assert_eq!(standard_color(35), Color::Magenta);
    assert_eq!(standard_color(36), Color::Cyan);
}

#[test]
fn an_unknown_code_changes_nothing() {
    assert_eq!(parse_color(&[7]), Format::default());
}

#[test]
fn a_long_run_of_digits_wraps_as_the_c_plus_plus_does() {
    // The accumulator is a `uint8_t`. 999 wraps to 231, which names no colour.
    assert_eq!(parse_color(&[231]), Format::default());
    let found = tokens("\u{1b}[999mtext");
    assert_eq!(found[0].fmt.expect("a format"), Format::default());
}

#[test]
fn an_overflowing_code_wraps_onto_a_real_colour() {
    // 287 wraps to 31, so a script printing `ESC[287m` gets red. Saturating
    // instead would give 255 and no colour at all, and with 999 the two are
    // indistinguishable -- a control was silent until this case existed. This
    // is nonsense input either way; it is pinned because the two readings
    // disagree about what the user sees.
    let found = tokens("\u{1b}[287mtext");

    assert_eq!(found[0].fmt.expect("a format").foreground, Some(Color::Red));
}

#[test]
fn the_cursor_reports_how_far_the_reader_got() {
    let mut tokenizer = Tokenizer::new("ab https://a.test x");
    tokenizer.next_token();

    assert_eq!(tokenizer.cursor(), 3);
}

#[test]
fn the_cursor_can_be_moved_for_output_that_arrives_in_pieces() {
    let mut tokenizer = Tokenizer::new("hidden:shown");
    tokenizer.set_cursor(7);

    assert_eq!(tokenizer.next_token().expect("a run").text, "shown");
}

#[test]
fn a_reader_at_the_end_has_nothing_more_to_say() {
    let mut tokenizer = Tokenizer::new("ab");
    tokenizer.set_cursor(2);

    assert!(tokenizer.next_token().is_none());
}
