//! The root search list's sections and selection.

use compass_core::root_items::{RootItem, RootItemMeta};
use compass_ui::root_list::{
    Move, RootList, Section, SectionKind, build, clamp_selection, favorite_rows, move_selection,
    selection_after_rebuild,
};

const NOW: i64 = 1_700_000_000;

fn item(id: &str, title: &str) -> RootItem {
    RootItem {
        id: format!("apps:{id}"),
        title: title.to_owned(),
        meta: RootItemMeta {
            provider_id: "apps".to_owned(),
            enabled: true,
            ..RootItemMeta::default()
        },
        ..RootItem::default()
    }
}

fn favorite(id: &str, title: &str, at: usize) -> RootItem {
    let mut it = item(id, title);
    it.meta.favorite_idx = Some(at);
    it
}

fn list(sections: &[(SectionKind, &[usize])]) -> RootList {
    RootList {
        sections: sections
            .iter()
            .map(|(kind, rows)| Section {
                kind: *kind,
                rows: rows.to_vec(),
            })
            .collect(),
    }
}

// --- headings -----------------------------------------------------------

#[test]
fn each_section_has_its_own_heading() {
    assert_eq!(SectionKind::Favorites.heading(), "Favorites");
    assert_eq!(SectionKind::Results.heading(), "Results");
    assert_eq!(SectionKind::Fallbacks.heading(), "Use with...");
    assert_eq!(SectionKind::Suggestions.heading(), "Suggestions");
}

#[test]
fn the_empty_query_and_a_search_are_headed_differently() {
    // "Suggestions" is a claim about why these rows are here; "Results" is a
    // claim about what was asked. Using one heading for both would say the
    // wrong thing in one of the two cases.
    assert_ne!(
        SectionKind::Suggestions.heading(),
        SectionKind::Results.heading()
    );
}

// --- what the list holds ------------------------------------------------

#[test]
fn headings_are_not_selectable_positions() {
    // Two sections of one row each is two positions, not four. This is the
    // number the selection arithmetic works in.
    let l = list(&[(SectionKind::Favorites, &[0]), (SectionKind::Results, &[1])]);
    assert_eq!(l.len(), 2);
}

#[test]
fn an_empty_list_has_nothing_to_select() {
    assert!(RootList::default().is_empty());
    assert_eq!(RootList::default().len(), 0);
}

#[test]
fn a_position_resolves_across_a_section_boundary() {
    let l = list(&[
        (SectionKind::Favorites, &[7, 8]),
        (SectionKind::Results, &[3]),
    ]);
    assert_eq!(l.item_at(0), Some(7));
    assert_eq!(l.item_at(1), Some(8));
    assert_eq!(l.item_at(2), Some(3));
    assert_eq!(l.item_at(3), None);
}

#[test]
fn a_position_reports_which_section_it_is_in() {
    let l = list(&[
        (SectionKind::Favorites, &[7, 8]),
        (SectionKind::Results, &[3]),
    ]);
    assert_eq!(l.locate(0), Some((0, 0)));
    assert_eq!(l.locate(1), Some((0, 1)));
    assert_eq!(l.locate(2), Some((1, 0)));
    assert_eq!(l.locate(3), None);
}

// --- building the list --------------------------------------------------

#[test]
fn favourites_lead_the_empty_query() {
    let items = vec![item("a", "Alpha"), favorite("b", "Beta", 0)];
    let built = build(&items, "", &[], NOW);
    assert_eq!(built.sections[0].kind, SectionKind::Favorites);
    assert_eq!(built.sections[0].rows, [1]);
}

#[test]
fn favourites_are_shown_in_the_order_they_were_arranged() {
    // Not by score: the whole point of arranging them is that the arrangement
    // is kept.
    let items = vec![
        favorite("a", "Alpha", 2),
        favorite("b", "Beta", 0),
        favorite("c", "Gamma", 1),
    ];
    assert_eq!(favorite_rows(&items), [1, 2, 0]);
}

#[test]
fn a_disabled_favourite_is_not_shown() {
    let mut items = vec![favorite("a", "Alpha", 0)];
    items[0].meta.enabled = false;
    assert!(favorite_rows(&items).is_empty());
}

#[test]
fn favourites_disappear_once_something_is_typed() {
    // A favourite that does not match is not an answer, and keeping it above
    // the results would push the thing the user asked for down the page.
    let items = vec![item("a", "Alpha"), favorite("b", "Beta", 0)];
    let built = build(&items, "alph", &[], NOW);
    assert!(
        !built
            .sections
            .iter()
            .any(|s| s.kind == SectionKind::Favorites),
        "{built:?}"
    );
}

#[test]
fn a_favourite_that_matches_appears_among_the_results() {
    // It is not hidden — it just loses its special position.
    let items = vec![item("a", "Alpha"), favorite("b", "Beta", 0)];
    let built = build(&items, "bet", &[], NOW);
    assert_eq!(built.sections.len(), 1);
    assert_eq!(built.sections[0].kind, SectionKind::Results);
    assert_eq!(built.sections[0].rows, [1]);
}

#[test]
fn a_favourite_appears_once_and_not_in_both_sections() {
    // With favourites excluded from the search for the empty query, they
    // appear under their own heading only. Including them would put the same
    // row on screen twice, which reads as two different things that happen to
    // share a name.
    let items = vec![item("a", "Alpha"), favorite("b", "Beta", 0)];
    let built = build(&items, "", &[], NOW);
    let shown: Vec<usize> = built.sections.iter().flat_map(|s| s.rows.clone()).collect();
    let mut unique = shown.clone();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(shown.len(), unique.len(), "a row appears twice: {built:?}");
    assert!(shown.contains(&1));
}

#[test]
fn the_empty_query_heads_its_matches_as_suggestions() {
    let items = vec![item("a", "Alpha")];
    let built = build(&items, "", &[], NOW);
    assert_eq!(built.sections[0].kind, SectionKind::Suggestions);
}

#[test]
fn a_search_heads_its_matches_as_results() {
    let items = vec![item("a", "Alpha")];
    let built = build(&items, "alph", &[], NOW);
    assert_eq!(built.sections[0].kind, SectionKind::Results);
}

#[test]
fn an_empty_section_is_never_drawn() {
    // A heading with nothing under it is a heading that lies.
    let items: Vec<RootItem> = Vec::new();
    assert!(build(&items, "", &[], NOW).sections.is_empty());
}

#[test]
fn fallbacks_appear_only_when_nothing_matched() {
    // Which is what makes them fallbacks rather than a section always on
    // screen.
    let items = vec![item("a", "Alpha")];
    let matched = build(&items, "alph", &[0], NOW);
    assert!(
        !matched
            .sections
            .iter()
            .any(|s| s.kind == SectionKind::Fallbacks)
    );

    let unmatched = build(&items, "zzzzz", &[0], NOW);
    assert_eq!(unmatched.sections.len(), 1);
    assert_eq!(unmatched.sections[0].kind, SectionKind::Fallbacks);
}

#[test]
fn nothing_matching_and_no_fallbacks_is_an_empty_list() {
    let items = vec![item("a", "Alpha")];
    assert!(build(&items, "zzzzz", &[], NOW).sections.is_empty());
}

#[test]
fn a_disabled_item_is_not_a_result() {
    let mut items = vec![item("a", "Alpha")];
    items[0].meta.enabled = false;
    assert!(build(&items, "alph", &[], NOW).sections.is_empty());
}

// --- moving the selection -----------------------------------------------

fn two_sections() -> RootList {
    list(&[
        (SectionKind::Favorites, &[0, 1]),
        (SectionKind::Results, &[2, 3]),
    ])
}

#[test]
fn the_selection_moves_down_one_row() {
    assert_eq!(move_selection(&two_sections(), 0, Move::Down, false), 1);
}

#[test]
fn the_selection_crosses_a_section_boundary_without_stopping() {
    // A heading is not a position, so nothing lands on one — and nothing is
    // skipped either. Position 1 is the last favourite and 2 is the first
    // result.
    let l = two_sections();
    assert_eq!(move_selection(&l, 1, Move::Down, false), 2);
    assert_eq!(l.item_at(2), Some(2));
}

#[test]
fn moving_back_up_crosses_the_boundary_the_same_way() {
    assert_eq!(move_selection(&two_sections(), 2, Move::Up, false), 1);
}

#[test]
fn walking_down_reaches_every_row_including_the_last() {
    // Testing one step from the top only would miss an off-by-one that makes
    // the final row unreachable — the list would look right and the last
    // result would never be selectable.
    let l = two_sections();
    let mut at = 0;
    let mut visited = vec![at];
    for _ in 0..l.len() {
        at = move_selection(&l, at, Move::Down, false);
        visited.push(at);
    }
    assert_eq!(visited, [0, 1, 2, 3, 3]);
}

#[test]
fn walking_up_reaches_the_first_row() {
    let l = two_sections();
    let mut at = l.len() - 1;
    for _ in 0..l.len() {
        at = move_selection(&l, at, Move::Up, false);
    }
    assert_eq!(at, 0);
}

#[test]
fn without_wrapping_the_ends_hold() {
    let l = two_sections();
    assert_eq!(move_selection(&l, 0, Move::Up, false), 0);
    assert_eq!(move_selection(&l, 3, Move::Down, false), 3);
}

#[test]
fn with_wrapping_the_ends_join() {
    let l = two_sections();
    assert_eq!(move_selection(&l, 3, Move::Down, true), 0);
    assert_eq!(move_selection(&l, 0, Move::Up, true), 3);
}

#[test]
fn an_empty_list_stays_at_zero_rather_than_wrapping_to_nothing() {
    let l = RootList::default();
    assert_eq!(move_selection(&l, 0, Move::Up, true), 0);
    assert_eq!(move_selection(&l, 0, Move::Down, true), 0);
}

#[test]
fn a_one_row_list_wraps_onto_itself() {
    let l = list(&[(SectionKind::Results, &[5])]);
    assert_eq!(move_selection(&l, 0, Move::Down, true), 0);
    assert_eq!(move_selection(&l, 0, Move::Up, true), 0);
}

// --- rebuilding ---------------------------------------------------------

#[test]
fn a_rebuilt_list_selects_its_first_row() {
    // A list rebuilt because the query changed is a different list; keeping
    // the old position would leave the selection on whatever happens to be
    // third now, which is how a launcher opens something nobody was looking
    // at.
    assert_eq!(selection_after_rebuild(), 0);
}

#[test]
fn a_selection_past_the_end_of_a_shrunken_list_is_pulled_back() {
    // A rebuild arriving while the user reads — a window closing, an index
    // finishing — must not leave the selection past the end.
    let l = list(&[(SectionKind::Results, &[0, 1])]);
    assert_eq!(clamp_selection(&l, 7), 1);
}

#[test]
fn a_selection_inside_the_list_is_left_alone() {
    let l = list(&[(SectionKind::Results, &[0, 1, 2])]);
    assert_eq!(clamp_selection(&l, 1), 1);
}

#[test]
fn a_selection_in_an_emptied_list_goes_to_zero() {
    assert_eq!(clamp_selection(&RootList::default(), 7), 0);
}
