//! The paste chord and the shortcut inhibitor on a real wlroots compositor:
//! headless Sway, a test window that records what its keyboard is sent, and
//! nothing touching a real device or session.

mod support;

use std::time::Duration;

use compass_wayland::keyboard_inhibit::{InhibitState, ShortcutInhibit};
use compass_wayland::virtual_keyboard::{
    CONTROL_MASK, KEY_LEFTCTRL, KEY_LEFTSHIFT, KEY_V, SHIFT_MASK, VirtualKeyboard,
};
use support::{KeyEvent, Sway, TestWindow, eventually};

const WAIT: Duration = Duration::from_secs(10);

/// The window's key presses, and the modifier mask in force at each.
fn chord(window: &TestWindow) -> Vec<(u32, bool, u32)> {
    let mut mask = 0;
    let mut chord = Vec::new();
    for event in window.presses() {
        match event {
            KeyEvent::Modifiers(now) => mask = now,
            KeyEvent::Key(code, pressed) => chord.push((code, pressed, mask)),
            _ => {}
        }
    }
    chord
}

fn focused(window: &TestWindow) -> bool {
    eventually(WAIT, || window.keys().contains(&KeyEvent::Enter))
}

#[test]
fn on_sway_the_paste_chord_reaches_the_focused_window() {
    let Some(sway) = Sway::start("on_sway_the_paste_chord_reaches_the_focused_window") else {
        return;
    };
    let _seat = support::seat_keyboard(&sway);
    let window = TestWindow::open(&sway, "Editor", "test.Editor");
    assert!(focused(&window), "{:?}", window.keys());

    VirtualKeyboard::bind(&sway.connect())
        .expect("a virtual keyboard")
        .paste(false)
        .expect("the chord was sent");

    let want = vec![
        (KEY_LEFTCTRL, true, 0),
        (KEY_V, true, CONTROL_MASK),
        (KEY_V, false, CONTROL_MASK),
        (KEY_LEFTCTRL, false, CONTROL_MASK),
    ];
    assert!(
        eventually(WAIT, || chord(&window) == want),
        "{:?}",
        window.keys()
    );
    assert!(
        eventually(WAIT, || window.presses().last()
            == Some(&KeyEvent::Modifiers(0))),
        "the modifiers were left held: {:?}",
        window.presses()
    );
    // The window was told what the codes mean: the chord's own keymap, as
    // the compositor serialises it again.
    assert!(
        window
            .keys()
            .iter()
            .any(|event| matches!(event, KeyEvent::Keymap(text) if text.lines().any(|line| line.contains("<AB04>") && line.trim_end().ends_with("= 55;")))),
        "{:?}",
        window.keys()
    );
}

#[test]
fn on_sway_a_terminal_is_sent_ctrl_shift_v() {
    let Some(sway) = Sway::start("on_sway_a_terminal_is_sent_ctrl_shift_v") else {
        return;
    };
    let _seat = support::seat_keyboard(&sway);
    let window = TestWindow::open(&sway, "Terminal", "test.Terminal");
    assert!(focused(&window));

    VirtualKeyboard::bind(&sway.connect())
        .expect("a virtual keyboard")
        .paste(true)
        .expect("the chord was sent");

    let both = CONTROL_MASK | SHIFT_MASK;
    let want = vec![
        (KEY_LEFTCTRL, true, 0),
        (KEY_LEFTSHIFT, true, CONTROL_MASK),
        (KEY_V, true, both),
        (KEY_V, false, both),
        (KEY_LEFTSHIFT, false, both),
        (KEY_LEFTCTRL, false, CONTROL_MASK),
    ];
    assert!(
        eventually(WAIT, || chord(&window) == want),
        "{:?}",
        window.keys()
    );
}

/// Polls `done`, taking the inhibitor's events as the launcher does: read
/// by someone else (here the window's thread, standing in for the toolkit)
/// and only dispatched here.
fn settle(inhibit: &mut ShortcutInhibit, done: impl Fn() -> bool) -> bool {
    eventually(WAIT, || {
        inhibit.dispatch_pending().expect("the connection holds");
        done()
    })
}

#[test]
fn on_sway_shortcuts_are_inhibited_on_the_focused_surface_while_wanted() {
    let Some(sway) = Sway::start("on_sway_shortcuts_are_inhibited_while_wanted") else {
        return;
    };
    let _seat = support::seat_keyboard(&sway);
    // The launcher's connection: the inhibitor and the surface share it, as
    // the launcher shares its connection with the toolkit.
    let connection = sway.connect();
    let mut inhibit = ShortcutInhibit::bind(&connection).expect("the inhibit manager");
    let handle = inhibit.handle();
    let window = TestWindow::open_on(&connection, "Launcher", "test.Launcher");

    assert!(
        settle(&mut inhibit, || handle.focused()),
        "{:?}",
        window.keys()
    );
    assert_eq!(handle.state(), InhibitState::None, "nothing until asked");

    inhibit.set_wanted(true);
    assert!(
        settle(&mut inhibit, || handle.state() == InhibitState::Active),
        "{:?}",
        handle.state()
    );
    assert_eq!(handle.made(), 1);

    inhibit.set_wanted(false);
    assert_eq!(handle.state(), InhibitState::None);

    // Asked again: a second inhibitor for the same surface and seat is a
    // protocol error, which would end the connection, so this also proves
    // the first was destroyed.
    inhibit.set_wanted(true);
    assert!(settle(&mut inhibit, || handle.state() == InhibitState::Active));
    assert_eq!(handle.made(), 2);
    inhibit.set_wanted(false);
    std::thread::sleep(Duration::from_millis(200));
    let error = connection.protocol_error();
    assert!(error.is_none(), "{error:?}");
    inhibit.dispatch_pending().expect("the connection holds");
}

#[test]
fn on_sway_the_inhibitor_goes_with_the_keyboard() {
    let Some(sway) = Sway::start("on_sway_the_inhibitor_goes_with_the_keyboard") else {
        return;
    };
    let _seat = support::seat_keyboard(&sway);
    let connection = sway.connect();
    let mut inhibit = ShortcutInhibit::bind(&connection).expect("the inhibit manager");
    let handle = inhibit.handle();
    // Wanted before anything is focused: nothing to inhibit yet.
    inhibit.set_wanted(true);
    assert_eq!(handle.state(), InhibitState::None);

    let _launcher = TestWindow::open_on(&connection, "Launcher", "test.Launcher");
    assert!(
        settle(&mut inhibit, || handle.state() == InhibitState::Active),
        "the surface that took the keyboard is inhibited: {:?}",
        handle.state()
    );

    // Another client's window takes the keyboard.
    let other = TestWindow::open(&sway, "Other", "test.Other");
    assert!(eventually(WAIT, || other.keys().contains(&KeyEvent::Enter)));
    assert!(
        settle(&mut inhibit, || !handle.focused()
            && handle.state() == InhibitState::None),
        "{:?}",
        handle.state()
    );
    assert!(handle.wanted(), "the wish outlives the focus");
}
