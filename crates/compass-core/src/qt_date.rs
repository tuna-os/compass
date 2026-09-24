//! Qt's date-format strings, for a snippet's `{date format="…"}`.
//!
//! A snippet stores its date format in Qt's syntax (`yyyy-MM-dd hh:mm`, the
//! C++ default), because `QDateTime::toString(format)` is what reads it. Those
//! strings are the user's data, so they have to keep meaning what they meant;
//! this formats a broken-down time with them.
//!
//! No crate formats Qt's syntax — `jiff`, `time` and `chrono` each have their
//! own — and a translation into one of theirs would still need this tokenizer,
//! so the formatter is here and the clock is the caller's (CRATE-AUDIT.md).
//!
//! The rules are Qt 6's: day and month names and the AM/PM marker are the C
//! locale's (English); `h`/`hh` are 12-hour only when the format has an AM/PM
//! marker; text in single quotes is literal and `''` is one quote; any other
//! character stands for itself.

/// A moment, broken down in the local time zone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DateTime {
    /// The year, e.g. 2026.
    pub year: i32,
    /// 1–12.
    pub month: u8,
    /// 1–31.
    pub day: u8,
    /// 0–23.
    pub hour: u8,
    /// 0–59.
    pub minute: u8,
    /// 0–59.
    pub second: u8,
    /// 0–999.
    pub millisecond: u16,
    /// 1 (Monday) to 7 (Sunday), as `QDate::dayOfWeek` counts.
    pub weekday: u8,
    /// The time zone's abbreviation, for `t`.
    pub zone: String,
}

const DAYS: [&str; 7] = [
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
    "Sunday",
];

const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

/// Formats `at` with the Qt format string `format`.
#[must_use]
pub fn format(format: &str, at: &DateTime) -> String {
    let chars: Vec<char> = format.chars().collect();
    let twelve_hour = has_am_pm(&chars);
    let mut out = String::with_capacity(format.len() + 8);
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '\'' {
            // `''` is a quote; otherwise everything up to the next quote is
            // literal, and an unclosed quote runs to the end.
            if chars.get(i + 1) == Some(&'\'') {
                out.push('\'');
                i += 2;
                continue;
            }
            i += 1;
            while i < chars.len() {
                if chars[i] == '\'' {
                    if chars.get(i + 1) == Some(&'\'') {
                        out.push('\'');
                        i += 2;
                        continue;
                    }
                    i += 1;
                    break;
                }
                out.push(chars[i]);
                i += 1;
            }
            continue;
        }
        let run = chars[i..].iter().take_while(|&&next| next == c).count();
        let used = token(c, run, at, twelve_hour, &chars[i..], &mut out);
        i += used.max(1);
    }
    out
}

/// Whether the format has an AM/PM marker outside quotes.
fn has_am_pm(chars: &[char]) -> bool {
    let mut quoted = false;
    for &c in chars {
        if c == '\'' {
            quoted = !quoted;
        } else if !quoted && matches!(c, 'a' | 'A') {
            return true;
        }
    }
    false
}

/// Writes the token starting with `run` copies of `c`, returning how many
/// characters it used; a character that starts no token is written as is.
fn token(
    c: char,
    run: usize,
    at: &DateTime,
    twelve_hour: bool,
    rest: &[char],
    out: &mut String,
) -> usize {
    use std::fmt::Write as _;
    let day_name = |long: bool| {
        let name = DAYS[usize::from(at.weekday.clamp(1, 7) - 1)];
        if long { name } else { &name[..3] }
    };
    let month_name = |long: bool| {
        let name = MONTHS[usize::from(at.month.clamp(1, 12) - 1)];
        if long { name } else { &name[..3] }
    };
    let hour12 = match at.hour % 12 {
        0 => 12,
        hour => hour,
    };
    match c {
        'y' if run >= 4 => {
            let _ = write!(out, "{:04}", at.year);
            4
        }
        'y' if run >= 2 => {
            let _ = write!(out, "{:02}", at.year.rem_euclid(100));
            2
        }
        'M' => {
            let used = run.min(4);
            match used {
                1 => {
                    let _ = write!(out, "{}", at.month);
                }
                2 => {
                    let _ = write!(out, "{:02}", at.month);
                }
                3 => out.push_str(month_name(false)),
                _ => out.push_str(month_name(true)),
            }
            used
        }
        'd' => {
            let used = run.min(4);
            match used {
                1 => {
                    let _ = write!(out, "{}", at.day);
                }
                2 => {
                    let _ = write!(out, "{:02}", at.day);
                }
                3 => out.push_str(day_name(false)),
                _ => out.push_str(day_name(true)),
            }
            used
        }
        'h' | 'H' => {
            let hour = if c == 'h' && twelve_hour {
                hour12
            } else {
                at.hour
            };
            let used = run.min(2);
            if used == 2 {
                let _ = write!(out, "{hour:02}");
            } else {
                let _ = write!(out, "{hour}");
            }
            used
        }
        'm' | 's' => {
            let value = if c == 'm' { at.minute } else { at.second };
            let used = run.min(2);
            if used == 2 {
                let _ = write!(out, "{value:02}");
            } else {
                let _ = write!(out, "{value}");
            }
            used
        }
        'z' => {
            if run >= 3 {
                let _ = write!(out, "{:03}", at.millisecond);
                3
            } else {
                // `z` is the milliseconds without trailing zeroes.
                let text = format!("{:03}", at.millisecond);
                let trimmed = text.trim_end_matches('0');
                out.push_str(if trimmed.is_empty() { "0" } else { trimmed });
                1
            }
        }
        'a' | 'A' => {
            let pm = at.hour >= 12;
            // `AP`/`ap` are two characters wide, `A`/`a` one; the case of the
            // first letter decides the marker's.
            let wide = matches!(rest.get(1), Some('p' | 'P'));
            let marker = if pm { "PM" } else { "AM" };
            if c == 'a' {
                out.push_str(&marker.to_lowercase());
            } else {
                out.push_str(marker);
            }
            if wide { 2 } else { 1 }
        }
        't' => {
            out.push_str(&at.zone);
            1
        }
        other => {
            out.push(other);
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at() -> DateTime {
        DateTime {
            year: 2026,
            month: 9,
            day: 4,
            hour: 15,
            minute: 7,
            second: 3,
            millisecond: 50,
            weekday: 5,
            zone: "CEST".to_owned(),
        }
    }

    #[test]
    fn the_cpp_default_format() {
        assert_eq!(format("yyyy-MM-dd hh:mm", &at()), "2026-09-04 15:07");
    }

    #[test]
    fn short_and_long_fields() {
        assert_eq!(format("d/M/yy", &at()), "4/9/26");
        assert_eq!(format("ddd d MMM", &at()), "Fri 4 Sep");
        assert_eq!(format("dddd, MMMM d", &at()), "Friday, September 4");
        assert_eq!(format("H:m:s.zzz t", &at()), "15:7:3.050 CEST");
        assert_eq!(format("s.z", &at()), "3.05");
    }

    #[test]
    fn am_pm_makes_h_twelve_hour() {
        assert_eq!(format("h:mm AP", &at()), "3:07 PM");
        assert_eq!(format("hh:mm ap", &at()), "03:07 pm");
        assert_eq!(format("HH:mm ap", &at()), "15:07 pm", "H stays 24-hour");
        let midnight = DateTime { hour: 0, ..at() };
        assert_eq!(format("h A", &midnight), "12 AM");
    }

    #[test]
    fn quoted_text_is_literal() {
        assert_eq!(format("'Week of' d MMM", &at()), "Week of 4 Sep");
        assert_eq!(format("h 'o''clock'", &at()), "15 o'clock");
        assert_eq!(format("''yy", &at()), "'26");
        assert_eq!(format("'unclosed d", &at()), "unclosed d");
    }

    #[test]
    fn a_single_y_is_just_a_letter() {
        assert_eq!(format("y yyy", &at()), "y 26y");
    }
}
