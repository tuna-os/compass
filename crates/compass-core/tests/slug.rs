//! What `slugify` guarantees about the names it produces.
//!
//! Read off `slugify` (`src/server/src/utils/utils.cpp`).

use compass_core::slug::slugify;

#[test]
fn an_empty_input_slugs_to_nothing() {
    assert_eq!(slugify(""), "");
}

#[test]
fn words_are_joined_by_the_separator() {
    assert_eq!(slugify("Simple List"), "simple-list");
}

#[test]
fn underscores_are_word_separators_too() {
    // The C++ pattern is `[\s_]+`, so an underscore is not punctuation to be
    // stripped but a space to be replaced.
    assert_eq!(slugify("my_command_name"), "my-command-name");
}

#[test]
fn a_run_of_whitespace_makes_one_separator_not_many() {
    assert_eq!(slugify("too   many    spaces"), "too-many-spaces");
}

#[test]
fn punctuation_is_removed_without_leaving_a_gap() {
    // `remove` deletes the match; it does not substitute a separator. So the
    // comma vanishes and the space beside it is what joins the words.
    assert_eq!(slugify("Hello, World!"), "hello-world");
}

#[test]
fn an_accented_letter_keeps_its_base_form() {
    // This is what the NFD pass buys: decompose first, and the strip removes
    // only the combining accent. Without it this would be "caf-au-lait".
    assert_eq!(slugify("Café au Lait"), "cafe-au-lait");
}

#[test]
fn leading_and_trailing_separators_are_trimmed() {
    assert_eq!(slugify("  padded title  "), "padded-title");
}

#[test]
fn separators_left_adjacent_by_stripping_are_collapsed() {
    // "a - b" becomes "a---b" after whitespace replacement, and the collapse
    // pass is the only thing that brings it back to one.
    assert_eq!(slugify("a - b"), "a-b");
}

#[test]
fn digits_survive() {
    assert_eq!(slugify("Version 2 Beta"), "version-2-beta");
}

#[test]
fn a_title_that_is_entirely_punctuation_slugs_to_nothing() {
    assert_eq!(slugify("!!! ??? !!!"), "");
}

#[test]
fn a_non_latin_title_is_transliterated_rather_than_emptied() {
    // A divergence from the C++, which strips these and leaves "" -- an
    // extension directory with no name. See PARITY.md.
    assert_eq!(slugify("日本語 ツール"), "ri-ben-yu-turu");
}
