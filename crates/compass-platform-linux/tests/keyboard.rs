//! What the virtual keyboard puts on the wire.
//!
//! Read off `linuxutils::UInputKeyboard` (`src/lib/linux-utils/`).

use std::time::Duration;

use compass_platform_linux::keyboard::{
    CharMap, DEFAULT_KEY_DELAY_US, EVDEV_OFFSET, EventSink, KEY_LEFTCTRL, KEY_LEFTSHIFT, KeyEvent,
    MODIFIER_DELAY_US, Modifiers, VirtualKeyboard, device,
};

/// A sink that remembers everything, so a test can read the wire.
#[derive(Debug, Default)]
struct Recorder {
    /// Every event, in order.
    events: Vec<KeyEvent>,
    /// Every wait, in order.
    waits: Vec<Duration>,
}

impl EventSink for Recorder {
    fn emit(&mut self, event: KeyEvent) {
        self.events.push(event);
    }

    fn delay(&mut self, duration: Duration) {
        self.waits.push(duration);
    }
}

/// A keyboard over a fresh recorder.
fn keyboard() -> VirtualKeyboard<Recorder> {
    VirtualKeyboard::new(Recorder::default())
}

/// A map where `a` is an unmodified key and `A` is that key with shift.
fn ascii_map() -> CharMap {
    let mut map = CharMap::new();
    map.set('a', 30, Modifiers::NONE);
    map.set('A', 30, Modifiers::SHIFT);
    map.set('b', 48, Modifiers::NONE);
    map
}

#[test]
fn a_bare_key_is_press_sync_release_sync() {
    let mut keyboard = keyboard();
    keyboard.send_key(30);
    assert_eq!(
        keyboard.sink().events,
        vec![
            KeyEvent::Press(30),
            KeyEvent::Sync,
            KeyEvent::Release(30),
            KeyEvent::Sync,
        ],
        "a reader that sees no sync after the press sees nothing at all"
    );
}

#[test]
fn a_modified_key_holds_the_modifier_across_the_whole_keystroke() {
    let mut keyboard = keyboard();
    keyboard.send_key_with_mods(30, Modifiers::SHIFT);

    let events = &keyboard.sink().events;
    let press_shift = events
        .iter()
        .position(|e| *e == KeyEvent::Press(KEY_LEFTSHIFT))
        .expect("shift pressed");
    let release_shift = events
        .iter()
        .position(|e| *e == KeyEvent::Release(KEY_LEFTSHIFT))
        .expect("shift released");
    let press_key = events
        .iter()
        .position(|e| *e == KeyEvent::Press(30))
        .expect("key pressed");
    let release_key = events
        .iter()
        .position(|e| *e == KeyEvent::Release(30))
        .expect("key released");

    assert!(press_shift < press_key, "shift must go down first");
    assert!(release_key < release_shift, "shift must come up last");
}

#[test]
fn ctrl_goes_down_before_shift_and_comes_up_in_the_same_order() {
    let mut keyboard = keyboard();
    keyboard.send_key_with_mods(30, Modifiers::CTRL.union(Modifiers::SHIFT));

    let events = &keyboard.sink().events;
    assert_eq!(events[0], KeyEvent::Press(KEY_LEFTCTRL));
    assert_eq!(events[1], KeyEvent::Press(KEY_LEFTSHIFT));

    let ctrl_up = events
        .iter()
        .position(|e| *e == KeyEvent::Release(KEY_LEFTCTRL));
    let shift_up = events
        .iter()
        .position(|e| *e == KeyEvent::Release(KEY_LEFTSHIFT));
    assert!(
        ctrl_up < shift_up,
        "clearMods releases in the same order applyMods presses, not in reverse"
    );
}

#[test]
fn four_of_the_six_modifiers_are_declared_and_never_applied() {
    // applyMods tests only Ctrl and Shift. A caller asking for Alt gets a bare
    // key, which is worth knowing about rather than discovering.
    for ignored in [
        Modifiers::ALT,
        Modifiers::LOGO,
        Modifiers::ALTGR,
        Modifiers::CAPSLOCK,
    ] {
        let mut keyboard = keyboard();
        keyboard.send_key_with_mods(30, ignored);
        let pressed: Vec<_> = keyboard
            .sink()
            .events
            .iter()
            .filter_map(|e| match e {
                KeyEvent::Press(code) => Some(*code),
                _ => None,
            })
            .collect();
        assert_eq!(
            pressed,
            vec![30],
            "{ignored:?} should press nothing but the key itself"
        );
    }
}

#[test]
fn an_ignored_modifier_still_chooses_the_longer_delay() {
    // The C++ branches on `mods ? MODIFIER_DELAY_US : m_keyDelayUs` — the raw
    // integer, before it decides what to do with it. So Alt slows the
    // keystroke down without modifying it.
    let mut keyboard = keyboard();
    keyboard.send_key_with_mods(30, Modifiers::ALT);
    let modifier_delay = Duration::from_micros(u64::from(MODIFIER_DELAY_US));
    assert!(
        keyboard.sink().waits.contains(&modifier_delay),
        "waits were {:?}",
        keyboard.sink().waits
    );
}

#[test]
fn an_unmodified_key_uses_the_ordinary_delay_throughout() {
    let mut keyboard = keyboard();
    keyboard.send_key_with_mods(30, Modifiers::NONE);
    let ordinary = Duration::from_micros(u64::from(DEFAULT_KEY_DELAY_US));
    assert!(
        keyboard.sink().waits.iter().all(|wait| *wait == ordinary),
        "waits were {:?}",
        keyboard.sink().waits
    );
}

#[test]
fn the_modifier_delay_is_five_times_the_ordinary_one() {
    // A compositor that reads the key before it has processed the modifier
    // types the unshifted character, which is why this gap exists at all.
    assert_eq!(DEFAULT_KEY_DELAY_US, 2000);
    assert_eq!(MODIFIER_DELAY_US, 10_000);
    assert_eq!(MODIFIER_DELAY_US, DEFAULT_KEY_DELAY_US * 5);
}

#[test]
fn a_configured_delay_replaces_the_ordinary_one_but_not_the_modifier_one() {
    let mut keyboard = keyboard();
    keyboard.set_key_delay_us(7);
    keyboard.send_key_with_mods(30, Modifiers::SHIFT);

    let waits = &keyboard.sink().waits;
    assert!(waits.contains(&Duration::from_micros(7)), "{waits:?}");
    assert!(
        waits.contains(&Duration::from_micros(u64::from(MODIFIER_DELAY_US))),
        "setKeyDelay does not touch MODIFIER_DELAY_US: {waits:?}"
    );
}

#[test]
fn repeat_key_sends_the_key_that_many_times() {
    let mut keyboard = keyboard();
    keyboard.repeat_key(30, 3);
    let presses = keyboard
        .sink()
        .events
        .iter()
        .filter(|e| **e == KeyEvent::Press(30))
        .count();
    assert_eq!(presses, 3);
}

#[test]
fn repeat_key_zero_times_sends_nothing() {
    let mut keyboard = keyboard();
    keyboard.repeat_key(30, 0);
    assert!(keyboard.sink().events.is_empty());
    assert!(keyboard.sink().waits.is_empty());
}

#[test]
fn repeat_key_pauses_after_the_last_press_too() {
    let mut keyboard = keyboard();
    keyboard.repeat_key(30, 2);
    assert_eq!(
        keyboard.sink().waits.len(),
        2,
        "one wait per press, not one fewer"
    );
}

#[test]
fn typing_uses_the_key_and_modifiers_the_map_gives() {
    let mut keyboard = keyboard();
    keyboard.set_char_map(ascii_map());
    keyboard.type_text("aA");

    let events = &keyboard.sink().events;
    let shifts = events
        .iter()
        .filter(|e| **e == KeyEvent::Press(KEY_LEFTSHIFT))
        .count();
    assert_eq!(shifts, 1, "only the capital needs shift");
    assert_eq!(
        events.iter().filter(|e| **e == KeyEvent::Press(30)).count(),
        2,
        "both characters are the same physical key"
    );
}

#[test]
fn a_character_the_map_cannot_type_is_dropped_and_the_rest_still_arrives() {
    // This is how a snippet with an em dash in it arrives with a hole rather
    // than not at all.
    let mut keyboard = keyboard();
    keyboard.set_char_map(ascii_map());
    keyboard.type_text("a—b");

    let presses: Vec<_> = keyboard
        .sink()
        .events
        .iter()
        .filter_map(|e| match e {
            KeyEvent::Press(code) => Some(*code),
            _ => None,
        })
        .collect();
    assert_eq!(presses, vec![30, 48], "the unmappable character is skipped");
}

#[test]
fn typing_with_an_empty_map_sends_nothing() {
    let mut keyboard = keyboard();
    keyboard.type_text("hello");
    assert!(keyboard.sink().events.is_empty());
}

#[test]
fn building_a_map_prefers_the_unshifted_binding() {
    // buildCharMap asks the unshifted question first and writes only into an
    // empty slot, so a character reachable both ways is typed without shift.
    let map = CharMap::build(|code| match code {
        30 => (Some('a'), Some('A')),
        31 => (Some('A'), Some('a')),
        _ => (None, None),
    });
    assert_eq!(map.get('a').expect("a is bound").mods, Modifiers::NONE);
    assert_eq!(map.get('a').expect("a is bound").code, 30);
}

#[test]
fn building_a_map_keeps_the_first_key_that_claims_a_character() {
    let map = CharMap::build(|code| match code {
        30 | 40 => (Some('/'), None),
        _ => (None, None),
    });
    assert_eq!(map.get('/').expect("bound").code, 30, "the lower key wins");
}

#[test]
fn a_shifted_binding_is_used_when_nothing_unshifted_claims_the_character() {
    let map = CharMap::build(|code| match code {
        30 => (Some('1'), Some('!')),
        _ => (None, None),
    });
    let record = map.get('!').expect("! is bound");
    assert_eq!(record.code, 30);
    assert_eq!(record.mods, Modifiers::SHIFT);
}

#[test]
fn a_non_ascii_binding_never_enters_the_map() {
    // The map is 128 slots indexed by byte, and the C++ only writes when
    // xkb_state_key_get_utf8 returns exactly one byte.
    let map = CharMap::build(|code| {
        if code == 30 {
            (Some('é'), None)
        } else {
            (None, None)
        }
    });
    assert_eq!(map.get('é'), None);
}

#[test]
fn the_evdev_offset_is_eight() {
    // xkb keycodes are evdev codes plus 8; getting this wrong shifts every
    // character by eight keys, which is a keyboard that types gibberish.
    assert_eq!(EVDEV_OFFSET, 8);
}

#[test]
fn the_device_announces_itself_the_way_the_cpp_does() {
    // A person looking for the device in /proc/bus/input/devices, or a
    // compositor rule matching on it, needs these to be unchanged.
    assert_eq!(device::NAME, "vicinae-snippet-virtual-keyboard");
    assert_eq!(device::BUSTYPE, 0x06);
    assert_eq!(device::VENDOR, 0x1234);
    assert_eq!(device::PRODUCT, 0x5678);
    assert_eq!(device::VERSION, 1);
}

#[test]
fn the_modifier_keycodes_are_the_evdev_ones() {
    // Asserted as literals, not against the constants the keyboard uses: a
    // test written in terms of the symbol passes whatever the symbol becomes,
    // and a wrong keycode here is a keyboard that holds some other key down.
    assert_eq!(KEY_LEFTCTRL, 29);
    assert_eq!(KEY_LEFTSHIFT, 42);
}

#[test]
fn a_shifted_keystroke_is_exactly_this_sequence() {
    // The whole wire, in order, including the sync after the key that
    // send_key already emitted. The C++ sends it twice; a reader counting
    // syncs can tell the two builds apart, so the extra one is pinned rather
    // than tidied away.
    let mut keyboard = keyboard();
    keyboard.send_key_with_mods(30, Modifiers::SHIFT);
    assert_eq!(
        keyboard.sink().events,
        vec![
            KeyEvent::Press(KEY_LEFTSHIFT),
            KeyEvent::Press(30),
            KeyEvent::Sync,
            KeyEvent::Release(30),
            KeyEvent::Sync,
            KeyEvent::Sync,
            KeyEvent::Release(KEY_LEFTSHIFT),
            KeyEvent::Sync,
        ]
    );
}

#[test]
fn an_unmodified_keystroke_through_send_key_with_mods_is_exactly_this_sequence() {
    let mut keyboard = keyboard();
    keyboard.send_key_with_mods(30, Modifiers::NONE);
    assert_eq!(
        keyboard.sink().events,
        vec![
            KeyEvent::Press(30),
            KeyEvent::Sync,
            KeyEvent::Release(30),
            KeyEvent::Sync,
            KeyEvent::Sync,
            KeyEvent::Sync,
        ],
        "even with no modifiers the C++ emits both trailing syncs"
    );
}

#[test]
fn a_key_that_types_the_same_character_shifted_or_not_is_recorded_unshifted() {
    // The previous test cannot see the order: with 'a' unshifted and 'A'
    // shifted, either order puts each character in its own empty slot. Only a
    // key whose two outputs are the *same* character forces the question, and
    // getting it wrong means typing 'a' while holding shift.
    let map = CharMap::build(|code| {
        if code == 30 {
            (Some('a'), Some('a'))
        } else {
            (None, None)
        }
    });
    assert_eq!(map.get('a').expect("a is bound").mods, Modifiers::NONE);
}
