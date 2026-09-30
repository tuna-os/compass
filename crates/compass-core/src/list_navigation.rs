//! Where the selection lands after a keypress.
//!
//! Ports `ListNavigation` (`src/server/src/services/navigation/list-navigation.hpp`),
//! which the C++ calls "the selection stepping semantics for every list-like
//! surface (root list, grids, action panel, completions)".
//!
//! # The default is to clamp, not to wrap
//!
//! `Config::wrapNavigation` is `false`, and the C++ returns the current index
//! unchanged when a step would run off either end. This crate's front end used
//! to wrap unconditionally, with a comment claiming that is "what every
//! launcher does"; the launcher being ported does not, so the comment was
//! wrong about the one launcher that mattered.
//!
//! Wrapping is a real option and stays available, behind the same setting the
//! C++ puts it behind.
//!
//! # Unselectable rows are stepped over, not landed on
//!
//! A list can hold things that are not rows — section headers, separators. The
//! C++ takes an `isSelectable` predicate and keeps stepping until it finds one
//! that is, giving up after `count` steps so that a list of nothing selectable
//! terminates. Both properties are reproduced.

/// Which way a step goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// Towards index 0.
    Backward,
    /// Towards the end.
    Forward,
}

/// The index a step from `from` lands on.
///
/// `count` is how many entries the list has; `wrap` is `Config::wrapNavigation`;
/// `selectable` answers whether an index can hold the selection.
///
/// Returns `from` when there is nowhere to go — an empty list, a step off the
/// end without wrapping, or a list with nothing selectable in it.
#[must_use]
pub fn next_index(
    from: usize,
    step: Step,
    count: usize,
    wrap: bool,
    selectable: impl Fn(usize) -> bool,
) -> usize {
    if count == 0 {
        return from;
    }

    let last = count - 1;
    let mut index = from;

    // At most `count` steps, so a list with nothing selectable stops instead
    // of circling for ever.
    for _ in 0..count {
        index = match step {
            Step::Forward => {
                if index >= last {
                    if !wrap {
                        return from;
                    }
                    0
                } else {
                    index + 1
                }
            }
            Step::Backward => {
                if index == 0 {
                    if !wrap {
                        return from;
                    }
                    last
                } else {
                    index - 1
                }
            }
        };

        if selectable(index) {
            return index;
        }
    }

    from
}

/// [`next_index`] over a list where every entry can be selected.
#[must_use]
pub fn next(from: usize, step: Step, count: usize, wrap: bool) -> usize {
    next_index(from, step, count, wrap, |_| true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamping_stops_at_both_ends() {
        assert_eq!(next(0, Step::Backward, 3, false), 0, "Up at the top stays");
        assert_eq!(next(2, Step::Forward, 3, false), 2, "Down at the end stays");
        assert_eq!(next(0, Step::Forward, 3, false), 1);
        assert_eq!(next(2, Step::Backward, 3, false), 1);
    }

    #[test]
    fn wrapping_goes_round_when_it_is_asked_for() {
        assert_eq!(next(0, Step::Backward, 3, true), 2);
        assert_eq!(next(2, Step::Forward, 3, true), 0);
    }

    #[test]
    fn an_empty_list_has_nowhere_to_go() {
        // Every caller uses the answer as an index, so this must not be a
        // number that is not there.
        assert_eq!(next(0, Step::Forward, 0, true), 0);
        assert_eq!(next(0, Step::Backward, 0, false), 0);
    }

    #[test]
    fn a_single_entry_list_stays_where_it_is() {
        assert_eq!(next(0, Step::Forward, 1, false), 0);
        assert_eq!(
            next(0, Step::Forward, 1, true),
            0,
            "wrapping onto itself is still itself"
        );
    }

    #[test]
    fn unselectable_entries_are_stepped_over() {
        // Index 1 is a section header: moving down from 0 must land on 2.
        let selectable = |index: usize| index != 1;
        assert_eq!(next_index(0, Step::Forward, 4, false, selectable), 2);
        assert_eq!(next_index(2, Step::Backward, 4, false, selectable), 0);
    }

    #[test]
    fn a_list_with_nothing_selectable_terminates() {
        // The `steps < count` bound. Without it this is an infinite loop with
        // wrapping on, which is the worst failure a keypress handler can have.
        assert_eq!(next_index(0, Step::Forward, 5, true, |_| false), 0);
        assert_eq!(next_index(3, Step::Backward, 5, true, |_| false), 3);
    }

    #[test]
    fn a_clamped_step_off_the_end_does_not_skip_to_a_selectable_entry() {
        // `if (!wrap) return from;` happens before the selectable check, so
        // the answer is where it started -- not the nearest selectable row in
        // the other direction.
        assert_eq!(next_index(2, Step::Forward, 3, false, |_| true), 2);
    }
}
