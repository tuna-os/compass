//! Keyboard focus over controls Iced cannot focus.
//!
//! Iced 0.14 moves focus only between text inputs: `focus_next` skips a
//! button, a switch and a dropdown, so a page of them cannot be reached
//! without a mouse. A page that has them keeps its own focus instead: the
//! controls it shows, in the order they are drawn, and which of them has the
//! keyboard. Tab and Shift+Tab walk that list; past either end the focus
//! goes back to the search field, which is where typing goes.
//!
//! A dropdown opened from the keyboard is a list of options under its row,
//! one of them highlighted, which the arrows move and Enter or Space picks.

/// The control after (or before) `current` in `controls`, or `None` for
/// the search field.
///
/// From the search field, Tab goes to the first control and Shift+Tab to
/// the last. A control that is no longer listed (its page changed under it)
/// counts as the search field.
#[must_use]
pub fn step<T: PartialEq + Clone>(controls: &[T], current: Option<&T>, forward: bool) -> Option<T> {
    let at = current.and_then(|current| controls.iter().position(|c| c == current));
    let next = match (at, forward) {
        (None, true) => 0,
        (None, false) => controls.len().checked_sub(1)?,
        (Some(at), true) => at + 1,
        (Some(at), false) => at.checked_sub(1)?,
    };
    controls.get(next).cloned()
}

/// A dropdown's highlighted option moved by `delta`, held to the list.
#[must_use]
pub fn move_highlight(len: usize, current: usize, delta: isize) -> usize {
    if len == 0 {
        return 0;
    }
    current.saturating_add_signed(delta).min(len - 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tab_walks_the_controls_and_comes_back_to_the_search_field() {
        let controls = ["a", "b", "c"];
        let mut at = None;
        let mut seen = Vec::new();
        for _ in 0..4 {
            at = step(&controls, at.as_ref(), true);
            seen.push(at);
        }
        assert_eq!(seen, [Some("a"), Some("b"), Some("c"), None]);
        assert_eq!(step(&controls, None, false), Some("c"));
        assert_eq!(step(&controls, Some(&"a"), false), None);
    }

    #[test]
    fn a_control_that_is_gone_counts_as_the_search_field() {
        assert_eq!(step(&["a"], Some(&"z"), true), Some("a"));
        assert_eq!(step::<&str>(&[], None, true), None);
        assert_eq!(step::<&str>(&[], None, false), None);
    }

    #[test]
    fn the_highlight_stays_in_the_list() {
        assert_eq!(move_highlight(3, 0, -1), 0);
        assert_eq!(move_highlight(3, 2, 1), 2);
        assert_eq!(move_highlight(3, 1, 1), 2);
        assert_eq!(move_highlight(0, 0, 1), 0);
    }
}
