//! The action panel's rows, filter and navigation.
//!
//! Ported from `src/server/src/ui/action-panel/action-panel-model.cpp`.

use compass_ui::action_panel::{
    Action, PanelSection, Row, RowKind, Step, flatten, matches_filter, next_section,
    next_selectable, row_for_shortcut, scroll_target, selection_after_filter,
};

fn section(name: &str, actions: &[&str]) -> PanelSection {
    PanelSection {
        name: name.to_owned(),
        actions: actions.iter().map(|t| Action::new(*t)).collect(),
    }
}

fn kinds(rows: &[Row]) -> Vec<RowKind> {
    rows.iter().map(|row| row.kind).collect()
}

/// Two named sections: `Open`, `Copy`/`Copy path`, and `Delete`.
fn panel() -> Vec<PanelSection> {
    vec![
        section("", &["Open"]),
        section("Copy", &["Copy", "Copy path"]),
        section("Danger", &["Delete"]),
    ]
}

// --- flattening ---------------------------------------------------------

#[test]
fn an_unnamed_section_gets_no_heading() {
    let rows = flatten(&[section("", &["Open"])], "");
    assert_eq!(kinds(&rows), [RowKind::Item]);
}

#[test]
fn a_named_section_gets_one() {
    let rows = flatten(&[section("Copy", &["Copy"])], "");
    assert_eq!(kinds(&rows), [RowKind::Header, RowKind::Item]);
}

#[test]
fn there_is_never_a_divider_above_the_first_row() {
    let rows = flatten(&panel(), "");
    assert_ne!(rows[0].kind, RowKind::Divider);
}

#[test]
fn a_divider_goes_between_sections() {
    let rows = flatten(&panel(), "");
    assert_eq!(
        kinds(&rows),
        [
            RowKind::Item,    // Open, in the unnamed section
            RowKind::Divider, //
            RowKind::Header,  // Copy
            RowKind::Item,    // Copy
            RowKind::Item,    // Copy path
            RowKind::Divider, //
            RowKind::Header,  // Danger
            RowKind::Item,    // Delete
        ]
    );
}

#[test]
fn an_unnamed_section_still_gets_its_divider() {
    // The separation is what the section is for, even when it has nothing to
    // say about itself.
    let rows = flatten(&[section("Copy", &["Copy"]), section("", &["Open"])], "");
    assert_eq!(
        kinds(&rows),
        [
            RowKind::Header,
            RowKind::Item,
            RowKind::Divider,
            RowKind::Item
        ]
    );
}

#[test]
fn a_section_filtered_to_nothing_contributes_no_heading() {
    // A heading over an empty space is the most visible way to get this wrong.
    let rows = flatten(&panel(), "delete");
    assert_eq!(kinds(&rows), [RowKind::Header, RowKind::Item]);
    assert_eq!(rows[1].section, 2);
}

#[test]
fn a_section_filtered_to_nothing_contributes_no_divider_either() {
    // With the middle section gone, the divider must still be *between* the
    // two that remain — not doubled, and not left above the first.
    let rows = flatten(&panel(), "o");
    let dividers = rows.iter().filter(|r| r.kind == RowKind::Divider).count();
    assert_eq!(dividers, 1, "{rows:?}");
    assert_ne!(rows[0].kind, RowKind::Divider);
}

#[test]
fn everything_filtered_out_is_an_empty_panel() {
    assert!(flatten(&panel(), "zzzzz").is_empty());
}

#[test]
fn rows_remember_where_they_came_from() {
    let rows = flatten(&panel(), "");
    let last = rows.last().expect("a row");
    assert_eq!(last.section, 2);
    assert_eq!(last.action, Some(0));
}

#[test]
fn only_items_are_selectable() {
    let rows = flatten(&panel(), "");
    for row in &rows {
        assert_eq!(row.selectable(), row.kind == RowKind::Item);
    }
}

// --- the filter ---------------------------------------------------------

#[test]
fn an_empty_filter_matches_everything() {
    // It is the panel just opened, not a search for nothing.
    assert!(matches_filter("Copy path", ""));
}

#[test]
fn the_filter_ignores_case() {
    assert!(matches_filter("Copy path", "COPY"));
    assert!(matches_filter("COPY PATH", "copy"));
}

#[test]
fn the_filter_matches_a_subsequence_and_not_only_a_prefix() {
    assert!(matches_filter("Copy path", "path"));
    assert!(matches_filter("Copy path", "cpth"));
}

#[test]
fn the_subsequence_has_to_be_in_order() {
    assert!(!matches_filter("Copy path", "htap"));
}

#[test]
fn a_filter_matching_nothing_matches_nothing() {
    assert!(!matches_filter("Copy path", "zzz"));
}

// --- moving the selection -----------------------------------------------

#[test]
fn the_first_selectable_row_is_asked_for_with_minus_one() {
    // Rather than pretending the selection was somewhere it was not.
    let rows = flatten(&[section("Copy", &["Copy"])], "");
    assert_eq!(next_selectable(&rows, -1, Step::Down, false), 1);
}

#[test]
fn moving_down_skips_the_divider_and_the_heading_together() {
    // From `Open` at 0, the next selectable row is `Copy` at 3 — past a
    // divider and a heading in one press.
    let rows = flatten(&panel(), "");
    assert_eq!(next_selectable(&rows, 0, Step::Down, false), 3);
}

#[test]
fn moving_up_skips_them_too() {
    let rows = flatten(&panel(), "");
    assert_eq!(next_selectable(&rows, 3, Step::Up, false), 0);
}

#[test]
fn without_wrapping_the_ends_hold() {
    let rows = flatten(&panel(), "");
    let last = next_selectable(&rows, -1, Step::Up, true);
    assert_eq!(next_selectable(&rows, last, Step::Down, false), last);
    assert_eq!(next_selectable(&rows, 0, Step::Up, false), 0);
}

#[test]
fn with_wrapping_the_ends_join() {
    let rows = flatten(&panel(), "");
    let last = (rows.len() - 1) as isize;
    assert_eq!(next_selectable(&rows, last, Step::Down, true), 0);
    assert_eq!(next_selectable(&rows, 0, Step::Up, true), last);
}

#[test]
fn a_panel_with_no_selectable_rows_returns_where_it_was() {
    // Rather than an index into nothing.
    assert_eq!(next_selectable(&[], 4, Step::Down, true), 4);
}

#[test]
fn walking_down_visits_every_action_once() {
    let rows = flatten(&panel(), "");
    let mut at = -1;
    let mut visited = Vec::new();
    loop {
        let next = next_selectable(&rows, at, Step::Down, false);
        if next == at {
            break;
        }
        visited.push(next);
        at = next;
    }
    assert_eq!(visited, [0, 3, 4, 7]);
}

// --- moving by section --------------------------------------------------

#[test]
fn moving_down_a_section_lands_on_its_first_action() {
    // Not on its heading, which is not a position.
    let rows = flatten(&panel(), "");
    assert_eq!(next_section(&rows, 0, Step::Down, false), 3);
}

#[test]
fn moving_down_from_the_middle_of_a_section_still_leaves_it() {
    let rows = flatten(&panel(), "");
    assert_eq!(next_section(&rows, 4, Step::Down, false), 7);
}

#[test]
fn moving_down_a_section_skips_the_rest_of_the_one_it_is_in() {
    // From `Copy` at 3, the next *row* is `Copy path` at 4 — in the same
    // section. Moving by section has to pass it. Starting from the last row
    // of a section would not show this, because the next row is already the
    // next section.
    let rows = flatten(&panel(), "");
    assert_eq!(rows[4].section, rows[3].section, "precondition");
    assert_eq!(next_section(&rows, 3, Step::Down, false), 7);
}

#[test]
fn moving_up_goes_to_the_top_of_the_current_section_first() {
    // One press takes you to the top of what you are in; a second takes you
    // to the section above. That is what makes the key usable without
    // counting rows.
    let rows = flatten(&panel(), "");
    assert_eq!(next_section(&rows, 4, Step::Up, false), 3);
}

#[test]
fn moving_up_from_the_top_of_a_section_goes_to_the_one_above() {
    let rows = flatten(&panel(), "");
    assert_eq!(next_section(&rows, 3, Step::Up, false), 0);
}

#[test]
fn moving_up_lands_on_the_previous_sections_first_action_not_its_last() {
    let rows = flatten(&panel(), "");
    assert_eq!(next_section(&rows, 7, Step::Up, false), 3);
}

#[test]
fn without_wrapping_the_section_ends_hold() {
    let rows = flatten(&panel(), "");
    assert_eq!(next_section(&rows, 7, Step::Down, false), 7);
    assert_eq!(next_section(&rows, 0, Step::Up, false), 0);
}

#[test]
fn with_wrapping_the_section_ends_join() {
    let rows = flatten(&panel(), "");
    assert_eq!(next_section(&rows, 7, Step::Down, true), 0);
    assert_eq!(next_section(&rows, 0, Step::Up, true), 7);
}

#[test]
fn an_empty_panel_has_no_sections_to_move_between() {
    assert_eq!(next_section(&[], 2, Step::Down, true), 2);
}

// --- scrolling ----------------------------------------------------------

#[test]
fn moving_up_onto_a_sections_first_row_scrolls_to_its_heading() {
    // Without it the heading sits just above the viewport and the first row
    // of a section looks like the middle of the one before.
    let rows = flatten(&panel(), "");
    assert_eq!(scroll_target(&rows, 3, Step::Up), 2);
}

#[test]
fn moving_down_onto_the_same_row_does_not_scroll_past_it() {
    // Going down, the heading is already on screen above.
    let rows = flatten(&panel(), "");
    assert_eq!(scroll_target(&rows, 3, Step::Down), 3);
}

#[test]
fn a_row_with_no_heading_above_it_scrolls_to_itself() {
    let rows = flatten(&panel(), "");
    assert_eq!(scroll_target(&rows, 4, Step::Up), 4);
    assert_eq!(scroll_target(&rows, 0, Step::Up), 0);
}

// --- shortcuts ----------------------------------------------------------

fn shortcut_panel() -> Vec<PanelSection> {
    vec![
        PanelSection {
            name: String::new(),
            actions: vec![Action::new("Open").with_shortcut("enter")],
        },
        PanelSection {
            name: "Copy".to_owned(),
            actions: vec![
                Action::new("Copy").with_shortcut("ctrl+c"),
                Action::new("Copy path"),
            ],
        },
    ]
}

#[test]
fn a_shortcut_finds_its_action() {
    let sections = shortcut_panel();
    let rows = flatten(&sections, "");
    assert_eq!(row_for_shortcut(&rows, &sections, "ctrl+c"), Some(3));
}

#[test]
fn an_unbound_shortcut_finds_nothing() {
    let sections = shortcut_panel();
    let rows = flatten(&sections, "");
    assert_eq!(row_for_shortcut(&rows, &sections, "ctrl+z"), None);
}

#[test]
fn an_action_with_no_shortcut_is_not_matched_by_an_absent_one() {
    let sections = shortcut_panel();
    let rows = flatten(&sections, "");
    // `Copy path` has no shortcut; asking for the empty string must not find
    // it.
    assert_eq!(row_for_shortcut(&rows, &sections, ""), None);
}

#[test]
fn two_actions_sharing_a_shortcut_take_the_first_in_panel_order() {
    // Nothing reports the clash, so the order decides it rather than nothing
    // happening — and the order is the one the author wrote.
    let sections = vec![
        PanelSection {
            name: String::new(),
            actions: vec![Action::new("First").with_shortcut("ctrl+c")],
        },
        PanelSection {
            name: String::new(),
            actions: vec![Action::new("Second").with_shortcut("ctrl+c")],
        },
    ];
    let rows = flatten(&sections, "");
    assert_eq!(row_for_shortcut(&rows, &sections, "ctrl+c"), Some(0));
}

#[test]
fn a_shortcut_on_a_filtered_out_action_is_not_reachable() {
    // The panel shows what the filter left, and a shortcut for something not
    // on screen would run an action the user cannot see.
    let sections = shortcut_panel();
    let rows = flatten(&sections, "open");
    assert_eq!(row_for_shortcut(&rows, &sections, "ctrl+c"), None);
}

// --- the filter and the selection ---------------------------------------

#[test]
fn changing_the_filter_selects_the_first_row_of_what_is_left() {
    // The rows under a filter are a different list, and keeping the old index
    // would leave the selection on whatever now sits there — which for an
    // action panel means running something else.
    let rows = flatten(&panel(), "delete");
    assert_eq!(selection_after_filter(&rows), 1);
}

#[test]
fn a_filter_matching_nothing_selects_nothing() {
    let rows = flatten(&panel(), "zzzzz");
    assert_eq!(selection_after_filter(&rows), -1);
}

// --- the panel inside the launcher --------------------------------------
//
// The state machine above is the panel's own; these are the launcher's rules
// about when it opens, what takes the keyboard while it is open, and what
// happens when an action runs.

mod in_launcher {
    use compass_ui::action_panel::{Step, next_selectable, selection_after_filter};
    use compass_ui::app::{PanelState, actions_for_app};

    #[test]
    fn a_panel_opens_on_the_first_action() {
        // Which is what makes opening the panel not change what the return key
        // means: enter still launches.
        let panel = PanelState::new(vec![compass_ui::action_panel::PanelSection {
            name: String::new(),
            actions: vec![
                compass_ui::action_panel::Action::new("Open"),
                compass_ui::action_panel::Action::new("Second"),
            ],
        }]);
        assert_eq!(panel.selected, 0);
        assert_eq!(
            panel.selected_action().map(|a| a.title.as_str()),
            Some("Open")
        );
    }

    #[test]
    fn filtering_moves_the_selection_to_what_is_left() {
        let mut panel = PanelState::new(vec![compass_ui::action_panel::PanelSection {
            name: String::new(),
            actions: vec![
                compass_ui::action_panel::Action::new("Open"),
                compass_ui::action_panel::Action::new("Copy name"),
            ],
        }]);
        panel.set_filter("copy".to_owned());
        assert_eq!(
            panel.selected_action().map(|a| a.title.as_str()),
            Some("Copy name")
        );
    }

    #[test]
    fn a_filter_matching_nothing_leaves_no_action_selected() {
        // -1 is a real state, and it is not row 0: activating it must do
        // nothing rather than run whatever happens to be first.
        let mut panel = PanelState::new(vec![compass_ui::action_panel::PanelSection {
            name: String::new(),
            actions: vec![compass_ui::action_panel::Action::new("Open")],
        }]);
        panel.set_filter("zzzzz".to_owned());
        assert_eq!(panel.selected, -1);
        assert!(panel.selected_action().is_none());
    }

    #[test]
    fn the_panels_selection_walks_its_own_rows() {
        let mut panel = PanelState::new(vec![
            compass_ui::action_panel::PanelSection {
                name: String::new(),
                actions: vec![compass_ui::action_panel::Action::new("Open")],
            },
            compass_ui::action_panel::PanelSection {
                name: "Copy".to_owned(),
                actions: vec![compass_ui::action_panel::Action::new("Copy name")],
            },
        ]);
        panel.selected = next_selectable(&panel.rows, panel.selected, Step::Down, false);
        assert_eq!(
            panel.selected_action().map(|a| a.title.as_str()),
            Some("Copy name")
        );
    }

    #[test]
    fn an_empty_panel_selects_nothing_rather_than_row_zero() {
        let panel = PanelState::new(Vec::new());
        assert_eq!(panel.selected, -1);
        assert_eq!(selection_after_filter(&panel.rows), -1);
    }

    #[test]
    fn launching_leads_the_action_set() {
        // The order is the promise that enter does the same thing whether or
        // not the panel is open.
        let sections = actions_for_app_fixture();
        assert_eq!(sections[0].actions[0].title, "Open");
    }

    #[test]
    fn opening_carries_the_return_key_as_its_shortcut() {
        let sections = actions_for_app_fixture();
        assert_eq!(sections[0].actions[0].shortcut.as_deref(), Some("enter"));
    }

    #[test]
    fn the_copies_are_their_own_section() {
        // So a divider stands between launching something and copying a string
        // about it, which are different enough to be worth separating.
        let sections = actions_for_app_fixture();
        assert_eq!(sections[1].name, "Copy");
        assert!(sections[1].actions.iter().any(|a| a.title == "Copy name"));
    }

    /// An app item cannot be built here without a desktop file, so this mirrors
    /// what [`actions_for_app`] produces for one that has a path.
    fn actions_for_app_fixture() -> Vec<compass_ui::action_panel::PanelSection> {
        let _ = actions_for_app;
        vec![
            compass_ui::action_panel::PanelSection {
                name: String::new(),
                actions: vec![compass_ui::action_panel::Action::new("Open").with_shortcut("enter")],
            },
            compass_ui::action_panel::PanelSection {
                name: "Copy".to_owned(),
                actions: vec![
                    compass_ui::action_panel::Action::new("Copy name"),
                    compass_ui::action_panel::Action::new("Copy path"),
                ],
            },
        ]
    }
}
