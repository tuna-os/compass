//! Robustness properties: the parser must never panic, hang or lose data on
//! arbitrary input.

use compass_xdg::{DesktopEntry, ExecParser, Locale, ParseOptions, Reader, value};
use proptest::prelude::*;

proptest! {
    #[test]
    fn parsing_never_panics(s in "\\PC*") {
        let _ = DesktopEntry::parse(&s);
    }
}

proptest! {
    #[test]
    fn parsing_arbitrary_lines_never_panics(s in "(\\PC|\n){0,400}") {
        let _ = DesktopEntry::parse(&s);
        let _ = Reader::parse(&s, Locale::parse("fr_FR@euro"));
    }
}

proptest! {
    #[test]
    fn parsing_desktop_shaped_input_never_panics(
        s in "(\\[[^\n]{0,20}\\]\n|[A-Za-z-]{0,10}(\\[[^\n\\]]{0,10}\\])?[ \t]*=?[^\n]{0,20}\n|#[^\n]{0,20}\n|\n){0,60}"
    ) {
        if let Ok(entry) = DesktopEntry::parse_with(
            &s,
            &ParseOptions { locale: Some(Locale::parse("de_DE@euro")), ..Default::default() },
        ) {
            let _ = entry.expand_exec_with(&["file:///tmp/a", "file:///tmp/b"], true, Some("env"));
            let _ = entry.should_show(&["GNOME"]);
            for action in entry.actions() {
                let _ = action.expand_exec();
            }
        }
    }
}

proptest! {
    #[test]
    fn locale_parsing_never_panics(s in "\\PC*") {
        let locale = Locale::parse(&s);
        // Re-parsing a rendered locale is stable.
        prop_assert_eq!(Locale::parse(&locale.to_string()), locale);
    }
}

proptest! {
    #[test]
    fn exec_expansion_never_panics(s in "\\PC*") {
        let _ = ExecParser::new("name")
            .with_icon(Some("icon"))
            .with_force_append(true)
            .parse(&s, &["uri"]);
    }
}

proptest! {
    #[test]
    fn value_conversions_never_panic(s in "\\PC*") {
        let _ = value::as_string(&s);
        let _ = value::as_bool(&s);
        let _ = value::as_number(&s);
        let _ = value::as_string_list(&s);
    }
}

proptest! {
    /// A value with no escape or separator round-trips through the file format.
    #[test]
    fn plain_values_round_trip(value in "[a-zA-Z0-9 _.+/-]{1,40}") {
        let trimmed = value.trim();
        prop_assume!(!trimmed.is_empty());

        let data = format!("[Desktop Entry]\nName={value}\n");
        let entry = DesktopEntry::parse_with(
            &data,
            &ParseOptions { locale: Some(Locale::parse("C")), ..Default::default() },
        )
        .unwrap();

        prop_assert_eq!(entry.name(), trimmed);
    }
}

proptest! {
    /// Escaping then unescaping a list is the identity.
    #[test]
    fn string_lists_round_trip(
        items in proptest::collection::vec(r"[^\\;\n]{0,10}", 1..6)
    ) {
        let encoded: String = items.iter().map(|item| format!("{item};")).collect();
        let decoded = value::as_string_list(&encoded);

        // Every element is terminated, so nothing is dropped -- including
        // empty elements, which only disappear after the last separator.
        prop_assert_eq!(decoded, items);
    }
}
