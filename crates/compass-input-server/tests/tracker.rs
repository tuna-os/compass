//! The key-sequence state machine, against recorded `evtest` sequences.
//!
//! The recordings in `tests/recordings/` are in `evtest`'s output format,
//! scan codes and `SYN_REPORT`s included, so what is replayed has the shape
//! the server reads from a real keyboard node. They were written in that
//! format for these cases, not captured from hardware; a capture from
//! `evtest` drops in beside them unchanged. They are replayed through a US
//! table ([`UsKeys`]) so they need no keymap on the machine; the xkbcommon
//! test at the end replays them through a compiled `us` keymap too, and
//! insists the two agree.

use std::path::Path;

use compass_core::input_server::wire::{Event, LayoutInfo};
use compass_core::snippet::ExpansionMode;
use compass_input_server::keymap::{UsKeys, XkbKeys};
use compass_input_server::recording::{self, EV_KEY};
use compass_input_server::tracker::{KeyState, Tracker};

fn recording(name: &str) -> Vec<recording::RawEvent> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/recordings")
        .join(name);
    let log = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    recording::parse(&log).unwrap()
}

/// Feeds every event the way the server does — only `EV_KEY` reaches the
/// tracker — and returns what it raised.
fn replay<K: KeyState>(tracker: &mut Tracker<K>, name: &str) -> Vec<Event> {
    recording(name)
        .into_iter()
        .filter(|event| event.kind == EV_KEY)
        .filter_map(|event| tracker.key(event.code, event.value))
        .collect()
}

fn tracker(snippets: &[(&str, ExpansionMode)]) -> Tracker<UsKeys> {
    let mut tracker = Tracker::new(UsKeys::new());
    for (trigger, mode) in snippets {
        tracker.add((*trigger).to_owned(), *mode);
    }
    tracker
}

fn trigger(text: &str) -> Event {
    Event::Trigger(text.to_owned())
}

#[test]
fn a_keydown_trigger_fires_on_its_last_key() {
    let mut tracker = tracker(&[(";sig", ExpansionMode::Keydown)]);
    assert_eq!(
        replay(&mut tracker, "keydown.evtest"),
        vec![trigger(";sig")]
    );
    assert_eq!(tracker.buffer(), "", "a match empties the buffer");
}

#[test]
fn a_word_trigger_waits_for_a_separator() {
    let mut tracker = tracker(&[(";sig", ExpansionMode::Word)]);
    assert!(replay(&mut tracker, "keydown.evtest").is_empty());

    for recording in ["word.evtest", "word_period.evtest"] {
        let mut tracker = self::tracker(&[(";sig", ExpansionMode::Word)]);
        assert_eq!(
            replay(&mut tracker, recording),
            vec![trigger(";sig")],
            "{recording}"
        );
    }
}

#[test]
fn tab_does_not_end_a_word() {
    let mut tracker = tracker(&[(";sig", ExpansionMode::Word)]);
    assert!(replay(&mut tracker, "word_tab.evtest").is_empty());
    assert_eq!(tracker.buffer(), ";sig", "the tab never reached the buffer");
}

#[test]
fn a_shifted_trigger_waits_for_shift_to_come_up() {
    // `:)` completes with Shift down; Ctrl+V injected now would arrive as
    // Ctrl+Shift+V. The event comes on Shift's release instead.
    let mut tracker = tracker(&[(":)", ExpansionMode::Keydown)]);
    let events: Vec<_> = recording("shifted.evtest")
        .into_iter()
        .filter(|event| event.kind == EV_KEY)
        .map(|event| {
            (
                event.code,
                event.value,
                tracker.key(event.code, event.value),
            )
        })
        .collect();
    let fired: Vec<_> = events
        .iter()
        .filter_map(|(code, value, event)| event.as_ref().map(|e| (*code, *value, e.clone())))
        .collect();
    assert_eq!(
        fired,
        vec![(42, 0, trigger(":)"))],
        "on Left Shift's release"
    );
    assert!(!tracker.has_pending());
}

#[test]
fn a_held_back_expansion_goes_when_the_wait_runs_out() {
    let mut tracker = tracker(&[(":)", ExpansionMode::Keydown)]);
    let events = recording("shifted.evtest");
    // Everything but the final Shift release: Shift is still down.
    for event in events.iter().filter(|e| e.kind == EV_KEY).take(5) {
        assert_eq!(tracker.key(event.code, event.value), None);
    }
    assert!(tracker.has_pending());
    assert_eq!(tracker.flush_pending(), Some(trigger(":)")));
    assert_eq!(tracker.flush_pending(), None, "once");
}

#[test]
fn backspace_straight_after_an_expansion_is_the_undo() {
    let mut tracker = tracker(&[(";sig", ExpansionMode::Keydown)]);
    assert_eq!(
        replay(&mut tracker, "undo.evtest"),
        vec![trigger(";sig"), Event::Undo(";sig".to_owned())]
    );
}

#[test]
fn any_other_press_disarms_the_undo_even_a_modifier() {
    let mut tracker = tracker(&[(";sig", ExpansionMode::Keydown)]);
    assert_eq!(
        replay(&mut tracker, "undo_disarmed.evtest"),
        vec![trigger(";sig")]
    );
}

#[test]
fn a_repeat_types_but_does_not_press() {
    let mut tracker = tracker(&[(";gg", ExpansionMode::Keydown)]);
    assert_eq!(replay(&mut tracker, "repeat.evtest"), vec![trigger(";gg")]);
    // The second repeat started a new buffer.
    assert_eq!(tracker.buffer(), "g");
}

#[test]
fn a_corrected_typo_still_fires() {
    let mut tracker = tracker(&[(";sig", ExpansionMode::Keydown)]);
    assert_eq!(
        replay(&mut tracker, "correction.evtest"),
        vec![trigger(";sig")]
    );
}

#[test]
fn control_chords_never_reach_the_buffer() {
    let mut tracker = tracker(&[(";sig", ExpansionMode::Keydown)]);
    assert!(replay(&mut tracker, "ctrl.evtest").is_empty());
    assert_eq!(tracker.buffer(), ";");
}

#[test]
fn a_reset_forgets_the_half_typed_trigger() {
    let mut tracker = tracker(&[(";sig", ExpansionMode::Keydown)]);
    let events: Vec<_> = recording("keydown.evtest")
        .into_iter()
        .filter(|event| event.kind == EV_KEY)
        .collect();
    let (before, after) = events.split_at(events.len() - 2);
    for event in before {
        assert_eq!(tracker.key(event.code, event.value), None);
    }
    tracker.reset();
    for event in after {
        assert_eq!(tracker.key(event.code, event.value), None);
    }
}

#[test]
fn a_removed_trigger_does_not_fire() {
    let mut tracker = tracker(&[(";sig", ExpansionMode::Keydown)]);
    assert!(tracker.remove(";sig"));
    assert!(replay(&mut tracker, "keydown.evtest").is_empty());
}

/// Every recording, through a compiled `us` keymap, gives what the table
/// gives. Skipped, loudly, where xkbcommon has no keymap data.
#[test]
fn xkbcommon_agrees_with_the_table() {
    let (mut keys, _) = match XkbKeys::new() {
        Ok(found) => found,
        Err(error) => {
            eprintln!("SKIPPED: {error} (is xkeyboard-config installed?)");
            return;
        }
    };
    let us = LayoutInfo {
        layout: "us".to_owned(),
        ..LayoutInfo::default()
    };
    let map = keys.set_layout(&us).expect("the us layout compiles");
    let table = UsKeys::char_map();
    for c in ' '..='~' {
        assert_eq!(map.get(c), table.get(c), "{c:?} is typed differently");
    }

    let cases: &[(&str, &str, ExpansionMode)] = &[
        ("keydown.evtest", ";sig", ExpansionMode::Keydown),
        ("word.evtest", ";sig", ExpansionMode::Word),
        ("word_period.evtest", ";sig", ExpansionMode::Word),
        ("word_tab.evtest", ";sig", ExpansionMode::Word),
        ("shifted.evtest", ":)", ExpansionMode::Keydown),
        ("undo.evtest", ";sig", ExpansionMode::Keydown),
        ("undo_disarmed.evtest", ";sig", ExpansionMode::Keydown),
        ("repeat.evtest", ";gg", ExpansionMode::Keydown),
        ("correction.evtest", ";sig", ExpansionMode::Keydown),
        ("ctrl.evtest", ";sig", ExpansionMode::Keydown),
    ];
    for (name, text, mode) in cases {
        let (mut xkb, _) = XkbKeys::new().unwrap();
        xkb.set_layout(&us).unwrap();
        let mut real = Tracker::new(xkb);
        real.add((*text).to_owned(), *mode);
        let mut table = tracker(&[(text, *mode)]);
        assert_eq!(
            replay(&mut real, name),
            replay(&mut table, name),
            "{name}: xkbcommon and the table disagree"
        );
        assert_eq!(real.buffer(), table.buffer(), "{name}: buffers differ");
    }
}

#[test]
fn an_unknown_layout_is_refused_and_the_old_one_kept() {
    let Ok((mut keys, _)) = XkbKeys::new() else {
        eprintln!("SKIPPED: xkbcommon has no keymap data here");
        return;
    };
    keys.set_layout(&LayoutInfo {
        layout: "us".to_owned(),
        ..LayoutInfo::default()
    })
    .unwrap();
    let bogus = LayoutInfo {
        layout: "no-such-layout-anywhere".to_owned(),
        ..LayoutInfo::default()
    };
    assert!(keys.set_layout(&bogus).is_err());
    assert_eq!(keys.text(30), "a", "still the old keymap");
}
