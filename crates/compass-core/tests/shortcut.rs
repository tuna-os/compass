//! Quicklink parsing, read against `Shortcut::parseLink` and
//! `Shortcut::insertPlaceholder` (`src/server/src/services/shortcut/shortcut.cpp`).

use compass_core::shortcut::{
    Argument, DEFAULT_APP_ID, Link, Placeholder, RESERVED_PLACEHOLDER_IDS, UrlPart, parse_link,
};

fn text(s: &str) -> UrlPart {
    UrlPart::Text(s.to_owned())
}

fn placeholder(id: &str, args: &[(&str, &str)]) -> UrlPart {
    UrlPart::Placeholder(Placeholder {
        id: id.to_owned(),
        args: args
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect(),
    })
}

fn argument(name: &str, default_value: &str) -> Argument {
    Argument {
        name: name.to_owned(),
        default_value: default_value.to_owned(),
    }
}

#[test]
fn a_link_without_placeholders_is_one_piece_of_text() {
    let link = parse_link("https://example.org");
    assert_eq!(link.parts, [text("https://example.org")]);
    assert!(link.placeholders.is_empty());
    assert!(link.arguments.is_empty());
    assert_eq!(link.raw, "https://example.org");
}

#[test]
fn a_bare_placeholder_is_an_argument_named_after_its_id() {
    // The comment on `m_reservedPlaceholderIds` says why: `?q={query}` is
    // shorthand for `?q={argument name="query"}`.
    let link = parse_link("https://example.org/search?q={query}");

    assert_eq!(
        link.parts,
        [
            text("https://example.org/search?q="),
            placeholder("query", &[])
        ]
    );
    assert_eq!(link.arguments, [argument("query", "")]);
}

#[test]
fn a_reserved_placeholder_is_not_an_argument() {
    // `isArgument = !isReserved || ...` -- a reserved id expands on its own and
    // the user is never asked for it.
    for id in RESERVED_PLACEHOLDER_IDS {
        let link = parse_link(&format!("https://example.org/{{{id}}}"));
        assert_eq!(link.placeholders.len(), 1, "{id}");
        assert!(link.arguments.is_empty(), "{id} should not be asked for");
    }
    assert_eq!(
        RESERVED_PLACEHOLDER_IDS,
        ["clipboard", "selection", "selected", "uuid", "date"]
    );
}

#[test]
fn a_named_argument_with_a_default_parses_both() {
    let link = parse_link(r#"https://example.org/{argument name="query" default="rust"}"#);

    assert_eq!(
        link.parts,
        [
            text("https://example.org/"),
            placeholder("argument", &[("default", "rust"), ("name", "query")]),
        ]
    );
    assert_eq!(link.arguments, [argument("query", "rust")]);
}

#[test]
fn a_value_needs_no_quotes_when_it_is_one_word() {
    // The `PhValue` state ends at the first character that is not a letter or
    // a number, so an unquoted value runs to the space or the closing brace.
    let link = parse_link("https://example.org/{argument name=query default=rust}");
    assert_eq!(link.arguments, [argument("query", "rust")]);
}

#[test]
fn a_quoted_value_may_contain_spaces_and_punctuation() {
    // `PhValueQuoted` only ends on the closing quote, so everything between is
    // kept -- which is the reason quoting exists here at all.
    let link = parse_link(r#"https://example.org/{argument name="search term" default="a, b"}"#);
    assert_eq!(link.arguments, [argument("search term", "a, b")]);
}

#[test]
fn the_name_argument_overrides_the_id() {
    let link = parse_link(r#"https://example.org/{q name="Search term"}"#);
    assert_eq!(link.arguments, [argument("Search term", "")]);
    assert_eq!(link.placeholders[0].id, "q", "the id is still the id");
}

#[test]
fn a_repeated_key_keeps_the_first_value() {
    // `std::map::insert` does not overwrite. Reproduced rather than improved:
    // a quicklink written with two `name=`s resolves the same on both engines.
    let link = parse_link("https://example.org/{argument name=first name=second}");
    assert_eq!(
        link.placeholders[0].args.get("name").map(String::as_str),
        Some("first")
    );
    assert_eq!(link.arguments, [argument("first", "")]);
}

#[test]
fn several_placeholders_keep_their_order_with_the_text_between_them() {
    let link = parse_link("https://{host}/search?q={query}&lang={lang}");

    assert_eq!(
        link.parts,
        [
            text("https://"),
            placeholder("host", &[]),
            text("/search?q="),
            placeholder("query", &[]),
            text("&lang="),
            placeholder("lang", &[]),
        ]
    );
    assert_eq!(
        link.arguments,
        [
            argument("host", ""),
            argument("query", ""),
            argument("lang", "")
        ]
    );
}

#[test]
fn an_unterminated_placeholder_swallows_the_rest_of_the_link() {
    // The trailing text is pushed only `if (state == BkNormal)`. A link still
    // being typed therefore shows the part before the brace and nothing else —
    // it is not an error, and the next keystroke may complete it.
    let link = parse_link("https://example.org/search?q={query");

    assert_eq!(link.parts, [text("https://example.org/search?q=")]);
    assert!(link.placeholders.is_empty());
    assert!(link.arguments.is_empty());
    assert_eq!(
        link.raw, "https://example.org/search?q={query",
        "though the raw link is kept whole"
    );
}

#[test]
fn an_empty_placeholder_is_an_argument_with_no_name() {
    // `{}`: the id ends immediately, the empty id is not reserved, so an
    // argument with an empty name is created. Pinned because it is what the
    // state machine does, not because it is useful.
    let link = parse_link("https://example.org/{}");

    assert_eq!(link.placeholders, [Placeholder::default()]);
    assert_eq!(link.arguments, [argument("", "")]);
}

#[test]
fn text_after_the_last_placeholder_is_kept() {
    let link = parse_link("https://example.org/{query}/page");
    assert_eq!(
        link.parts,
        [
            text("https://example.org/"),
            placeholder("query", &[]),
            text("/page"),
        ]
    );
}

#[test]
fn the_default_app_id_is_the_string_default() {
    // `Shortcut::defaultAppId()` -- what `isDefaultApp` compares against, and
    // what a quicklink stores when the user has not picked an application.
    assert_eq!(DEFAULT_APP_ID, "default");
}

#[test]
fn an_empty_link_parses_to_nothing() {
    assert_eq!(parse_link(""), Link::default());
}
