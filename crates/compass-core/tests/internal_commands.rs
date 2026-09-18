//! The internal commands and the Markdown showcase document.
//!
//! Ported from `src/server/src/builtins/internal/`.

use compass_core::internal_commands::{
    CLOSE_ACTION_LABEL, EXTENSION_ID, EXTENSION_NAME, SHOWCASE, registered_commands,
};

#[test]
fn the_extension_holds_one_command() {
    assert_eq!(EXTENSION_ID, "internal");
    assert_eq!(registered_commands(), ["markdown-showcase"]);
}

#[test]
fn the_extension_is_named_the_same_way_twice() {
    // The C++ returns the same string for the display name and the
    // description. It is not meant to be found by searching, so a description
    // that distinguished it from its own name would be describing it to
    // nobody.
    assert_eq!(EXTENSION_NAME, "Internal Commands");
}

#[test]
fn the_showcase_closes_rather_than_doing_anything() {
    assert_eq!(CLOSE_ACTION_LABEL, "Close");
}

// --- what the document must cover ---------------------------------------
//
// The showcase is a test fixture that ships. Its whole value is coverage, so
// these tests enumerate what it has to contain: dropping a construct while
// editing the prose would otherwise go unnoticed until someone looked at a
// heading that had quietly stopped being one.

#[test]
fn every_heading_level_the_renderer_supports_appears() {
    for (level, text) in [
        ("# ", "Heading 1"),
        ("## ", "Heading 2"),
        ("### ", "Heading 3"),
        ("#### ", "Heading 4"),
        ("##### ", "Heading 5"),
    ] {
        assert!(
            SHOWCASE.contains(&format!("{level}{text}")),
            "missing {text}"
        );
    }
}

#[test]
fn the_inline_styles_all_appear() {
    assert!(SHOWCASE.contains("**bold text**"));
    assert!(SHOWCASE.contains("*italic text*"));
    assert!(SHOWCASE.contains("***bold italic***"));
    assert!(SHOWCASE.contains("`inline code`"));
    assert!(SHOWCASE.contains("~~strikethrough~~"));
}

#[test]
fn both_kinds_of_line_break_appear() {
    // A soft break and a hard break render differently, and only one of them
    // is visible in the source, so both are spelled out in the document.
    assert!(SHOWCASE.contains("soft break"));
    assert!(SHOWCASE.contains("Line before hard break\\"));
    assert!(SHOWCASE.contains("Line after hard break"));
}

#[test]
fn a_link_appears() {
    assert!(SHOWCASE.contains("[link to Vicinae](https://vicinae.com)"));
}

#[test]
fn every_labelled_code_language_appears() {
    // The label drives the syntax highlighting, so each one is a separate
    // thing that can break.
    for lang in ["cpp", "python", "javascript", "rust", "bash", "json"] {
        assert!(SHOWCASE.contains(&format!("```{lang}")), "missing {lang}");
    }
}

#[test]
fn an_unlabelled_code_block_appears_too() {
    // A fence with no language is its own path through the highlighter.
    assert!(SHOWCASE.contains("```\n"));
}

#[test]
fn both_kinds_of_list_appear() {
    assert!(SHOWCASE.contains("### Unordered List"));
    assert!(SHOWCASE.contains("### Ordered List"));
    assert!(SHOWCASE.contains("- First item"));
    assert!(SHOWCASE.contains("1. First step"));
}

#[test]
fn a_list_item_carries_inline_formatting() {
    // Inline styles inside a list item are a different case from inline
    // styles in a paragraph.
    assert!(SHOWCASE.contains("- Second item with **bold**"));
    assert!(SHOWCASE.contains("- Third item with `inline code`"));
}

#[test]
fn a_table_with_all_three_alignments_appears() {
    // Left, centre and right are three separate pieces of the table renderer,
    // and the separator row is the only thing that distinguishes them.
    assert!(SHOWCASE.contains("|---------|:------:|------:|"));
}

#[test]
fn a_table_cell_carries_inline_formatting() {
    assert!(SHOWCASE.contains("| **Bold** text |"));
}

#[test]
fn both_shapes_of_blockquote_appear() {
    assert!(SHOWCASE.contains("> This is a plain blockquote."));
    assert!(SHOWCASE.contains("> This is a multi-paragraph blockquote."));
    // The empty quoted line is what makes the second paragraph a paragraph.
    assert!(SHOWCASE.contains("\n>\n"));
}

#[test]
fn every_callout_kind_appears() {
    // Each kind has its own colour and glyph, so a missing one is a missing
    // code path rather than a missing sentence.
    for kind in ["NOTE", "TIP", "IMPORTANT", "WARNING", "CAUTION"] {
        assert!(
            SHOWCASE.contains(&format!("> [!{kind}]")),
            "missing the {kind} callout"
        );
    }
}

#[test]
fn a_plain_image_and_a_linked_image_both_appear() {
    // An image wrapped in a link is not the same node as an image, and the
    // showcase covers both.
    assert!(SHOWCASE.contains("![HTTP GIF]("));
    assert!(SHOWCASE.contains("[![Pikachu]("));
}

#[test]
fn a_horizontal_rule_appears() {
    assert!(SHOWCASE.contains("\n---\n"));
}

#[test]
fn the_document_ends_where_it_says_it_does() {
    // The last line is the only way to see, from the rendered page, that
    // nothing was truncated.
    assert!(SHOWCASE.trim_end().ends_with("*End of markdown showcase.*"));
}

#[test]
fn the_document_was_copied_rather_than_retyped() {
    // A fixture whose job is to exercise a renderer is exactly where a
    // character typed wrong still looks right in review. These are the
    // measurements of the C++ raw string literal.
    assert_eq!(SHOWCASE.len(), 3060);
    assert_eq!(SHOWCASE.lines().count(), 146);
}
