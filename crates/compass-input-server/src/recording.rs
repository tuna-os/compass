//! Reading `evtest` output, so key sequences can be replayed in tests.
//!
//! `evtest /dev/input/eventN` prints one line per event:
//!
//! ```text
//! Event: time 1727180000.000100, type 1 (EV_KEY), code 30 (KEY_A), value 1
//! Event: time 1727180000.000100, -------------- SYN_REPORT ------------
//! ```
//!
//! Recording a real keyboard is `evtest --grab` free: run `evtest`, pick the
//! keyboard, type, and save what it printed. The replay keeps every event
//! (`EV_MSC` scan codes and `SYN_REPORT`s included), so the server's filtering
//! is exercised on what a keyboard really sends, not on a tidied list.

/// `EV_SYN`.
pub const EV_SYN: u16 = 0;
/// `EV_KEY`.
pub const EV_KEY: u16 = 1;
/// `EV_MSC`.
pub const EV_MSC: u16 = 4;

/// One event, as the kernel delivers it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RawEvent {
    /// The event type (`EV_KEY`, `EV_MSC`, …).
    pub kind: u16,
    /// The code within the type.
    pub code: u16,
    /// The value: for keys, 0 release, 1 press, 2 repeat.
    pub value: i32,
}

/// Parses an `evtest` log. Lines that are not events (the device banner,
/// comments starting with `#`, blanks) are skipped.
///
/// # Errors
///
/// When an `Event:` line has a `type` but its numbers do not parse, with the
/// line number.
pub fn parse(log: &str) -> Result<Vec<RawEvent>, String> {
    let mut events = Vec::new();
    for (number, line) in log.lines().enumerate() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("Event:") else {
            continue;
        };
        if rest.contains("SYN_REPORT") && !rest.contains("type") {
            events.push(RawEvent {
                kind: EV_SYN,
                code: 0,
                value: 0,
            });
            continue;
        }
        let field = |name: &str| -> Option<&str> {
            let start = rest.find(&format!("{name} "))? + name.len() + 1;
            let rest = &rest[start..];
            let end = rest
                .find(|c: char| !(c.is_ascii_alphanumeric() || c == '-'))
                .unwrap_or(rest.len());
            Some(&rest[..end])
        };
        let parsed = (|| {
            let kind: u16 = field("type")?.parse().ok()?;
            let value = field("value")?;
            // evtest prints `EV_MSC` values (scan codes) in hex, the rest in
            // decimal.
            let value = if kind == EV_MSC {
                u32::from_str_radix(value, 16).ok()?.cast_signed()
            } else {
                value.parse().ok()?
            };
            Some(RawEvent {
                kind,
                code: field("code")?.parse().ok()?,
                value,
            })
        })();
        match parsed {
            Some(event) => events.push(event),
            None => return Err(format!("line {}: not an event: {line}", number + 1)),
        }
    }
    Ok(events)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_evtest_log_parses_to_its_events() {
        let log = "\
Input driver version is 1.0.1
Event: time 1727180000.000100, type 4 (EV_MSC), code 4 (MSC_SCAN), value 7002a
Event: time 1727180000.000100, type 1 (EV_KEY), code 30 (KEY_A), value 1
Event: time 1727180000.000100, -------------- SYN_REPORT ------------
# a comment
Event: time 1727180000.090000, type 1 (EV_KEY), code 30 (KEY_A), value 0
";
        assert_eq!(
            parse(log).unwrap(),
            vec![
                RawEvent {
                    kind: EV_MSC,
                    code: 4,
                    value: 0x7002a
                },
                RawEvent {
                    kind: EV_KEY,
                    code: 30,
                    value: 1
                },
                RawEvent {
                    kind: EV_SYN,
                    code: 0,
                    value: 0
                },
                RawEvent {
                    kind: EV_KEY,
                    code: 30,
                    value: 0
                },
            ]
        );
    }

    #[test]
    fn a_mangled_event_line_is_an_error() {
        assert!(parse("Event: time 1.0, type x (EV_KEY), code 30, value 1").is_err());
    }
}
