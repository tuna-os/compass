//! What words a filename is findable by.
//!
//! Read off `file_indexer::vocab`
//! (`src/file-indexer/src/file-indexer/vocabulary.hpp`).

use compass_core::vocabulary::{
    MAX_TOKEN_LENGTH, MIN_HEX_JUNK_LENGTH, MIN_TOKEN_LENGTH, basename, dirname, file_extension,
    is_junk_token, is_skeleton_vowel, skeletonize_token, tokenize_filename,
};

#[test]
fn the_limits_are_the_cpp_ones() {
    assert_eq!(MIN_TOKEN_LENGTH, 3);
    assert_eq!(MAX_TOKEN_LENGTH, 24);
    assert_eq!(MIN_HEX_JUNK_LENGTH, 12);
}

#[test]
fn a_filename_splits_into_its_words() {
    assert_eq!(
        tokenize_filename("annual-report-final.pdf"),
        vec!["annual", "report", "final", "pdf"]
    );
}

#[test]
fn camel_case_is_a_word_boundary() {
    assert_eq!(
        tokenize_filename("annualReportFinal"),
        vec!["annual", "report", "final"]
    );
}

#[test]
fn an_acronym_does_not_swallow_the_word_after_it() {
    // The rule that makes XMLParser two tokens: an uppercase letter whose
    // predecessor is uppercase and whose successor is lowercase ends the
    // acronym. Without it the whole thing is one token nobody will type.
    assert_eq!(tokenize_filename("XMLParser"), vec!["xml", "parser"]);
    assert_eq!(
        tokenize_filename("HTTPSConnection"),
        vec!["https", "connection"]
    );
}

#[test]
fn a_run_of_capitals_at_the_end_stays_one_token() {
    // No lowercase letter follows, so there is no acronym boundary to find.
    assert_eq!(tokenize_filename("parseXML"), vec!["parse", "xml"]);
    assert_eq!(tokenize_filename("XMLHTTP"), vec!["xmlhttp"]);
}

#[test]
fn a_digit_before_a_capital_is_a_boundary() {
    assert_eq!(
        tokenize_filename("report2024Final"),
        vec!["report2024", "final"]
    );
}

#[test]
fn tokens_come_out_lowercased() {
    assert_eq!(tokenize_filename("README"), vec!["readme"]);
}

#[test]
fn a_two_letter_word_is_not_worth_a_row() {
    // It would match far too much.
    assert_eq!(tokenize_filename("my-report"), vec!["report"]);
    assert_eq!(tokenize_filename("ab"), Vec::<String>::new());
    assert_eq!(tokenize_filename("abc"), vec!["abc"], "three is enough");
}

#[test]
fn an_all_digit_token_is_dropped() {
    // A year or a serial number matches too many files to be useful.
    assert_eq!(tokenize_filename("report-2024.pdf"), vec!["report", "pdf"]);
    assert_eq!(tokenize_filename("12345"), Vec::<String>::new());
}

#[test]
fn a_digit_inside_a_word_is_kept() {
    assert_eq!(tokenize_filename("mp3player"), vec!["mp3player"]);
}

#[test]
fn a_content_hash_is_junk() {
    // The thing that fills a build directory or a git object store.
    assert!(is_junk_token("a1b2c3d4e5f6"));
    assert!(is_junk_token("deadbeefcafebabe"));
    assert_eq!(
        tokenize_filename("blob-a1b2c3d4e5f6.bin"),
        vec!["blob", "bin"]
    );
}

#[test]
fn a_short_hex_looking_word_is_not_junk() {
    // The twelve-character floor is what keeps the rule from eating real
    // words, all of which are hex-shaped.
    for word in ["deface", "facade", "added", "decade", "beefed"] {
        assert!(!is_junk_token(word), "{word} should survive");
    }
}

#[test]
fn a_hex_string_one_character_short_of_the_floor_survives() {
    assert!(!is_junk_token("a1b2c3d4e5f"), "eleven characters");
    assert!(is_junk_token("a1b2c3d4e5f6"), "twelve");
}

#[test]
fn a_hex_string_with_a_non_hex_letter_is_not_junk() {
    // "g" is not a hex digit, so this is a word that happens to look like one.
    assert!(!is_junk_token("a1b2c3d4e5fg"));
}

#[test]
fn anything_longer_than_the_limit_is_junk() {
    // Past 24 bytes it is a hash, a base64 blob or a concatenation — never
    // something a person will type.
    // Built from a non-hex letter, or the hash rule would reject it first and
    // the length rule would go untested.
    assert!(is_junk_token(&"z".repeat(MAX_TOKEN_LENGTH + 1)));
    assert!(!is_junk_token(&"z".repeat(MAX_TOKEN_LENGTH)));
}

#[test]
fn a_non_ascii_filename_survives_intact() {
    // Bytes above 0x7F are token characters, so a name in another script is
    // one token rather than being split at every byte — and it must come back
    // as the characters it went in as.
    let tokens = tokenize_filename("résumé");
    assert_eq!(tokens, vec!["résumé"]);
}

#[test]
fn a_mixed_script_name_splits_at_the_punctuation_only() {
    let tokens = tokenize_filename("報告書-final.pdf");
    assert_eq!(tokens, vec!["報告書", "final", "pdf"]);
}

#[test]
fn a_skeleton_drops_vowels_but_keeps_the_first_letter() {
    // That first letter is what stops "apple" and "ripple" colliding.
    assert_eq!(skeletonize_token("apple"), "apl");
    assert_eq!(skeletonize_token("ripple"), "rpl");
    assert_ne!(skeletonize_token("apple"), skeletonize_token("ripple"));
}

#[test]
fn a_typo_shares_its_skeleton_with_the_word() {
    // Which is the whole point: a misspelling still finds the file.
    assert_eq!(skeletonize_token("apple"), skeletonize_token("aple"));
    assert_eq!(skeletonize_token("apple"), skeletonize_token("appel"));
}

#[test]
fn a_doubled_consonant_collapses() {
    assert_eq!(skeletonize_token("cattle"), "ctl");
    assert_eq!(skeletonize_token("catle"), "ctl");
}

#[test]
fn a_skeleton_is_lowercase() {
    assert_eq!(skeletonize_token("APPLE"), skeletonize_token("apple"));
}

#[test]
fn a_word_starting_with_a_vowel_keeps_it() {
    assert!(skeletonize_token("orange").starts_with('o'));
    assert!(skeletonize_token("elephant").starts_with('e'));
}

#[test]
fn the_skeleton_vowels_are_the_five() {
    for vowel in b"aeiou" {
        assert!(is_skeleton_vowel(*vowel));
    }
    assert!(!is_skeleton_vowel(b'y'), "y is not one of them here");
    assert!(!is_skeleton_vowel(b'A'), "the check is on lowercase bytes");
}

#[test]
fn an_empty_token_skeletonises_to_nothing() {
    assert_eq!(skeletonize_token(""), "");
}

#[test]
fn the_basename_is_everything_after_the_last_slash() {
    assert_eq!(basename("/home/ada/notes.txt"), "notes.txt");
    assert_eq!(basename("notes.txt"), "notes.txt");
    assert_eq!(basename("/home/ada/"), "");
}

#[test]
fn the_dirname_is_everything_before_it() {
    assert_eq!(dirname("/home/ada/notes.txt"), "/home/ada");
    assert_eq!(
        dirname("notes.txt"),
        "notes.txt",
        "no slash, so no directory"
    );
}

#[test]
fn the_extension_is_everything_after_the_last_dot() {
    assert_eq!(file_extension("notes.txt"), "txt");
    assert_eq!(file_extension("archive.tar.gz"), "gz");
}

#[test]
fn a_file_with_no_dot_reports_its_own_name_as_an_extension() {
    // What the C++ returns, and the callers treat it as a hint rather than a
    // fact — so reproducing it is safer than inventing an empty answer they
    // have never seen.
    assert_eq!(file_extension("Makefile"), "Makefile");
}

#[test]
fn an_empty_name_tokenises_to_nothing() {
    assert_eq!(tokenize_filename(""), Vec::<String>::new());
    assert_eq!(tokenize_filename("---"), Vec::<String>::new());
}

#[test]
fn a_realistic_filename_gives_the_words_somebody_would_type() {
    assert_eq!(
        tokenize_filename("2024-Q3_SalesReport_FINAL_v2.xlsx"),
        vec!["sales", "report", "final", "xlsx"]
    );
}
