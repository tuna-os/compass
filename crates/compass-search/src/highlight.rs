//! Where search terms occur in a text, for highlighting: `MatchHighlighter`.
//!
//! A port of `src/server/src/ui/quick/match-highlighter.cpp`. Each term is
//! found literally — not fuzzily — in the text, case- and diacritic-
//! insensitively, every occurrence, left to right without overlapping itself.
//! The fold keeps one character for one character (as the C++'s does per
//! UTF-16 unit), so a match's position in the folded text is its position
//! in the text as written.

use std::ops::Range;

fn fold(c: char) -> char {
    let folded = crate::matcher::fold(c);
    let mut lower = folded.to_lowercase();
    match (lower.next(), lower.next()) {
        (Some(one), None) => one,
        _ => folded,
    }
}

/// The byte ranges of `text` that one of `terms` matches, sorted and with
/// overlapping or touching ranges merged; empty terms match nothing.
#[must_use]
pub fn term_ranges(text: &str, terms: &[&str]) -> Vec<Range<usize>> {
    let chars: Vec<(usize, char)> = text.char_indices().map(|(at, c)| (at, fold(c))).collect();
    let end_of = |index: usize| chars.get(index).map_or(text.len(), |(at, _)| *at);
    let mut found: Vec<Range<usize>> = Vec::new();
    for term in terms {
        let term: Vec<char> = term.chars().map(fold).collect();
        if term.is_empty() || term.len() > chars.len() {
            continue;
        }
        let mut from = 0;
        while from + term.len() <= chars.len() {
            let matched = chars[from..from + term.len()]
                .iter()
                .map(|(_, c)| *c)
                .eq(term.iter().copied());
            if matched {
                found.push(chars[from].0..end_of(from + term.len()));
                from += term.len();
            } else {
                from += 1;
            }
        }
    }
    found.sort_by_key(|range| range.start);
    let mut merged: Vec<Range<usize>> = Vec::with_capacity(found.len());
    for range in found {
        match merged.last_mut() {
            Some(last) if range.start <= last.end => last.end = last.end.max(range.end),
            _ => merged.push(range),
        }
    }
    merged
}

#[cfg(test)]
mod tests {
    use super::*;

    fn marked(text: &str, terms: &[&str]) -> String {
        let mut out = String::new();
        let mut at = 0;
        for range in term_ranges(text, terms) {
            out.push_str(&text[at..range.start]);
            out.push('[');
            out.push_str(&text[range.clone()]);
            out.push(']');
            at = range.end;
        }
        out.push_str(&text[at..]);
        out
    }

    #[test]
    fn every_occurrence_of_each_term_is_found_ignoring_case_and_accents() {
        assert_eq!(
            marked("Café au lait, CAFE noir", &["cafe"]),
            "[Café] au lait, [CAFE] noir"
        );
        assert_eq!(
            marked("résumé.pdf and resume.txt", &["resume", "txt"]),
            "[résumé].pdf and [resume].[txt]"
        );
    }

    #[test]
    fn a_term_does_not_overlap_itself_and_overlapping_terms_merge() {
        assert_eq!(marked("aaaa", &["aa"]), "[aaaa]");
        assert_eq!(marked("aaa", &["aa"]), "[aa]a");
        assert_eq!(
            marked("hello world", &["hello w", "o wor"]),
            "[hello wor]ld"
        );
    }

    #[test]
    fn nothing_is_marked_without_terms_or_a_match() {
        assert_eq!(term_ranges("anything", &[]), []);
        assert_eq!(term_ranges("anything", &[""]), []);
        assert_eq!(term_ranges("short", &["much longer"]), []);
        assert_eq!(term_ranges("", &["a"]), []);
    }
}
