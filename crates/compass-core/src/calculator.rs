//! The calculator in root search.
//!
//! The C++ engine ships Numen, its own in-tree calculator; this uses
//! [`fend_core`], an existing Rust one with no dependencies of its own, which
//! covers arithmetic, percentages and unit conversions. Currency conversion
//! needs exchange rates, which it has no source for yet.
//!
//! When to try is the C++ rule (`RootSearchModel::refreshCalculator`): a query
//! starting with `=` always, otherwise only one of at least
//! [`MIN_CHARS`] characters that matched nothing else. One rule is added,
//! because fend reads almost any word as something (`a` is one ampere, `sin`
//! is the sine function): without the `=`, a query must contain a digit.

use std::time::{Duration, Instant};

/// The shortest query tried without a leading `=`.
pub const MIN_CHARS: usize = 3;

/// How long one evaluation may take before it is abandoned. A query is typed
/// a keystroke at a time; an answer later than this is worse than none.
const TIME_LIMIT: Duration = Duration::from_millis(50);

/// A calculation and its result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Answer {
    /// The expression, as evaluated.
    pub question: String,
    /// The result, as fend prints it (`4`, `1.524 m`, `approx. 3.1415926536`).
    pub answer: String,
}

struct Deadline(Instant);

impl fend_core::Interrupt for Deadline {
    fn should_interrupt(&self) -> bool {
        Instant::now() >= self.0
    }
}

/// The answer to `query`, if it should be shown: `found_other` says whether
/// the root search found anything else.
#[must_use]
pub fn evaluate(query: &str, found_other: bool) -> Option<Answer> {
    let query = query.trim();
    let question = match query.strip_prefix('=') {
        Some(forced) => forced.trim(),
        None => {
            if found_other
                || query.chars().count() < MIN_CHARS
                || !query.chars().any(|c| c.is_ascii_digit())
            {
                return None;
            }
            query
        }
    };
    if question.is_empty() {
        return None;
    }
    let mut context = fend_core::Context::new();
    let result = fend_core::evaluate_with_interrupt(
        question,
        &mut context,
        &Deadline(Instant::now() + TIME_LIMIT),
    )
    .ok()?;
    let answer = result.get_main_result().trim();
    // A lone number, or a function name, answers nothing.
    if result.output_is_empty() || answer.is_empty() || answer == question {
        return None;
    }
    Some(Answer {
        question: question.to_owned(),
        answer: answer.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn answer(query: &str) -> Option<String> {
        evaluate(query, false).map(|a| a.answer)
    }

    #[test]
    fn arithmetic_percentages_and_units_are_answered() {
        assert_eq!(answer("2+2*3").as_deref(), Some("8"));
        assert_eq!(answer("10% of 200").as_deref(), Some("20"));
        assert_eq!(answer("5 ft to m").as_deref(), Some("1.524 m"));
        assert_eq!(answer("0.1+0.2").as_deref(), Some("0.3"));
    }

    #[test]
    fn words_short_queries_and_other_matches_are_left_alone() {
        assert_eq!(answer("firefox"), None);
        assert_eq!(answer("sin"), None, "a function name is not a question");
        assert_eq!(answer("1+"), None, "too short");
        assert_eq!(answer("12345"), None, "a number answers nothing");
        assert_eq!(evaluate("2+2+2", true), None, "something else matched");
        assert_eq!(answer("1/0"), None, "an error is no answer");
    }

    #[test]
    fn a_leading_equals_forces_it() {
        assert_eq!(
            evaluate("=pi", true).map(|a| a.answer).as_deref(),
            Some("approx. 3.1415926536")
        );
        assert_eq!(
            evaluate("= 1+1", true).map(|a| a.question).as_deref(),
            Some("1+1")
        );
        assert_eq!(evaluate("=", true), None);
    }

    #[test]
    fn a_slow_expression_is_abandoned_not_waited_for() {
        let started = Instant::now();
        let _ = evaluate("=2^2^2^2^2^2", false);
        assert!(started.elapsed() < Duration::from_secs(2));
    }
}
