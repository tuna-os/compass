//! The calculator history view's own decisions.
//!
//! Ports `src/server/src/builtins/calculator/` — when the search box is also
//! treated as a calculator, which icon a remembered answer gets, and what the
//! action panel offers. The arithmetic and the persistence are elsewhere: the
//! backends are unported by design, and the rows live in
//! `compass_local_storage::calculator`.

/// How many characters the search box needs before it is also read as a sum.
///
/// Below this the box is only a search box. The threshold exists because
/// almost any two characters parse as *something*, and a result flickering
/// into the list on the way to typing a word is worse than no result.
pub const CALCULATOR_MIN_CHARS: usize = 3;

/// The prefix that asks for a calculation whatever the length.
pub const EXPLICIT_PREFIX: char = '=';

/// What, if anything, the search text should be handed to the calculator as.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LiveCalc {
    /// Do not compute; the box is only a search box.
    None,
    /// Compute this text.
    Compute(String),
}

/// Decide whether the search text is also a calculation, and what of it.
///
/// A leading `=` asks explicitly, and is stripped before the rest is computed.
/// It also bypasses the length rule entirely, so `=1` computes although `1`
/// alone would not — which is the point of having the prefix.
///
/// The length is counted in characters. A bare `=` asks for a calculation of
/// the empty string; the C++ hands that to the backend too, and lets the
/// backend decline.
#[must_use]
pub fn live_calc(text: &str) -> LiveCalc {
    let explicit = text.starts_with(EXPLICIT_PREFIX);
    if !explicit && text.chars().count() < CALCULATOR_MIN_CHARS {
        return LiveCalc::None;
    }
    let body = if explicit {
        text.chars().skip(1).collect()
    } else {
        text.to_owned()
    };
    LiveCalc::Compute(body)
}

/// How the live result is written into the list.
///
/// One row, `question = answer`, with the spaces around the sign — the
/// separator is part of the title rather than something the view draws, so it
/// is here and not in the theme.
#[must_use]
pub fn live_result_title(question: &str, answer: &str) -> String {
    format!("{question} = {answer}")
}

/// The icon a remembered row shows.
///
/// A conversion gets the switch glyph and everything else the calculator one —
/// the C++ `switch` has a `default`, so an unknown type hint is drawn as
/// ordinary arithmetic rather than left blank.
#[must_use]
pub fn history_icon(is_conversion: bool) -> &'static str {
    if is_conversion {
        "switch"
    } else {
        "calculator"
    }
}

/// One entry in the action panel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Action {
    /// What it does.
    pub id: &'static str,
    /// What it is called, where it has a label of its own.
    pub title: Option<&'static str>,
    /// Whether it is the one the return key runs.
    pub primary: bool,
}

/// The action panel for a remembered row, as sections.
///
/// The sections are the grouping the panel draws separators between, so the
/// shape is part of the behaviour: pinning on its own, then the three copies,
/// then the two destructive actions kept away from the primary action by a
/// separator rather than by a confirmation.
///
/// Copying the *answer* is primary, not copying the question — the question is
/// what was typed and the answer is what is wanted.
#[must_use]
pub fn history_action_panel(is_pinned: bool) -> Vec<Vec<Action>> {
    vec![
        vec![Action {
            id: if is_pinned { "unpin" } else { "pin" },
            title: None,
            primary: false,
        }],
        vec![
            Action {
                id: "copy-answer",
                title: Some("Copy answer"),
                primary: true,
            },
            Action {
                id: "copy-question",
                title: Some("Copy question"),
                primary: false,
            },
            Action {
                id: "copy-expression",
                title: Some("Copy question and answer"),
                primary: false,
            },
        ],
        vec![
            Action {
                id: "remove",
                title: None,
                primary: false,
            },
            Action {
                id: "remove-all",
                title: None,
                primary: false,
            },
        ],
    ]
}

/// The action panel for the live result.
///
/// The unformatted answer only appears when the backend produced one — a sum
/// whose answer is already unformatted does not get a second, identical copy
/// action.
#[must_use]
pub fn live_action_panel(has_unformatted: bool) -> Vec<Vec<Action>> {
    let mut main = vec![
        Action {
            id: "copy-answer",
            title: None,
            primary: true,
        },
        Action {
            id: "copy-question-and-answer",
            title: None,
            primary: false,
        },
    ];
    if has_unformatted {
        main.push(Action {
            id: "copy-unformatted-answer",
            title: Some("Copy unformatted answer"),
            primary: false,
        });
    }
    vec![main]
}

/// Drop the groups with nothing in them.
///
/// The grouping produces every group on every call; this is the view's half of
/// that split, and keeping it here rather than in the grouping is what lets a
/// caller rely on the full set of groups being present.
pub fn visible_groups<T>(groups: Vec<(String, Vec<T>)>) -> Vec<(String, Vec<T>)> {
    groups
        .into_iter()
        .filter(|(_, records)| !records.is_empty())
        .collect()
}
