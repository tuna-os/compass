//! When the paste service copies, when it waits, and when it gives up.
//!
//! Read off `PasteService` (`src/server/src/services/paste/paste-service.cpp`).

use std::cell::RefCell;

use compass_core::paste::{
    BLIND_PASTE_DELAY_MS, FOCUS_POLL_INTERVAL_MS, FOCUS_POLL_MAX, POST_FOCUS_DELAY_MS,
    PasteEnvironment, PasteService, PasteTarget, Window, focus_timeout_ms,
};

/// Everything the service did, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Did {
    Copy(String),
    StartPolling,
    StopPolling,
    ScheduleExecute(u64),
    Paste(PasteTarget),
    RestoreClipboard,
}

/// A world the tests drive by hand.
struct World {
    copies: bool,
    pastes: bool,
    detects_handoff: bool,
    /// Focus answers, consumed one per poll; `None` means focus has not landed.
    foreign: RefCell<Vec<Option<Window>>>,
    /// What `getFocusedWindow` answers at paste time.
    focused: Option<Window>,
    /// The app the database knows, by class.
    app: Option<(String, String)>,
    did: RefCell<Vec<Did>>,
}

impl Default for World {
    fn default() -> Self {
        Self {
            copies: true,
            pastes: true,
            detects_handoff: true,
            foreign: RefCell::new(Vec::new()),
            focused: None,
            app: None,
            did: RefCell::new(Vec::new()),
        }
    }
}

impl World {
    /// Focus never lands, however long we poll.
    fn focus_never_lands(self) -> Self {
        self
    }

    /// Focus lands after `n` polls.
    fn focus_lands_after(self, n: usize) -> Self {
        let mut answers = vec![None; n];
        answers.push(Some(a_window()));
        *self.foreign.borrow_mut() = answers;
        self
    }
}

impl PasteEnvironment for World {
    type Content = String;

    fn copy(&self, content: &String) -> bool {
        self.did.borrow_mut().push(Did::Copy(content.clone()));
        self.copies
    }
    fn supports_paste(&self) -> bool {
        self.pastes
    }
    fn supports_focus_handoff_detection(&self) -> bool {
        self.detects_handoff
    }
    fn focused_foreign_window(&self) -> Option<Window> {
        let mut answers = self.foreign.borrow_mut();
        if answers.is_empty() {
            return None;
        }
        answers.remove(0)
    }
    fn focused_window(&self) -> Option<Window> {
        self.focused.clone()
    }
    fn find_app(&self, wm_class: &str) -> Option<String> {
        self.app
            .as_ref()
            .filter(|(class, _)| class == wm_class)
            .map(|(_, app)| app.clone())
    }
    fn schedule_execute(&self, delay_ms: u64) {
        self.did.borrow_mut().push(Did::ScheduleExecute(delay_ms));
    }
    fn start_focus_polling(&self) {
        self.did.borrow_mut().push(Did::StartPolling);
    }
    fn stop_focus_polling(&self) {
        self.did.borrow_mut().push(Did::StopPolling);
    }
    fn paste_to_app(&self, target: &PasteTarget) -> bool {
        self.did.borrow_mut().push(Did::Paste(target.clone()));
        true
    }
    fn schedule_clipboard_restore(&self) {
        self.did.borrow_mut().push(Did::RestoreClipboard);
    }
}

/// A window to paste into.
fn a_window() -> Window {
    Window {
        title: "untitled — Editor".to_owned(),
        wm_class: "org.editor.Editor".to_owned(),
    }
}

/// What the service did.
fn did(service: &PasteService<World>) -> Vec<Did> {
    service.env().did.borrow().clone()
}

#[test]
fn the_timings_are_the_cpp_constants() {
    assert_eq!(FOCUS_POLL_INTERVAL_MS, 5);
    assert_eq!(FOCUS_POLL_MAX, 1000);
    assert_eq!(POST_FOCUS_DELAY_MS, 30);
    assert_eq!(BLIND_PASTE_DELAY_MS, 150);
    assert_eq!(
        focus_timeout_ms(),
        5000,
        "five seconds, as the comment says"
    );
}

#[test]
fn a_paste_copies_first_and_then_starts_polling() {
    let mut service = PasteService::new(World::default().focus_lands_after(2));
    assert!(service.paste_content(&"hello".to_owned()));

    assert_eq!(
        did(&service),
        vec![
            Did::Copy("hello".to_owned()),
            // The in-flight cancel happens even with nothing in flight.
            Did::StopPolling,
            Did::StartPolling,
        ]
    );
    assert!(service.has_pending_paste());
}

#[test]
fn a_copy_that_fails_stops_everything() {
    let mut service = PasteService::new(World {
        copies: false,
        ..World::default()
    });
    assert!(!service.paste_content(&"hello".to_owned()));
    assert_eq!(did(&service), vec![Did::Copy("hello".to_owned())]);
    assert!(!service.has_pending_paste());
}

#[test]
fn a_platform_that_cannot_paste_still_gets_the_copy() {
    // The C++ copies before it asks. Reproduced deliberately: the content is
    // on the clipboard and the person can paste it themselves, and a caller
    // that retried on false would otherwise copy twice.
    let mut service = PasteService::new(World {
        pastes: false,
        ..World::default()
    });
    assert!(!service.paste_content(&"hello".to_owned()));
    assert_eq!(
        did(&service),
        vec![Did::Copy("hello".to_owned())],
        "copied, but nothing scheduled"
    );
    assert!(!service.has_pending_paste());
}

#[test]
fn without_focus_detection_the_paste_is_scheduled_blind() {
    let mut service = PasteService::new(World {
        detects_handoff: false,
        ..World::default()
    });
    assert!(service.paste_content(&"hello".to_owned()));
    assert_eq!(
        did(&service),
        vec![
            Did::Copy("hello".to_owned()),
            Did::StopPolling,
            Did::ScheduleExecute(BLIND_PASTE_DELAY_MS),
        ],
        "no signal to wait on, so it guesses long"
    );
}

#[test]
fn the_blind_delay_is_longer_than_the_post_focus_one() {
    // Not a coincidence: one is "focus has landed, give it a moment", the
    // other is "we have no idea, hope for the best".
    const { assert!(BLIND_PASTE_DELAY_MS > POST_FOCUS_DELAY_MS) };
}

#[test]
fn focus_landing_stops_the_poll_and_schedules_the_paste() {
    let mut service = PasteService::new(World::default().focus_lands_after(2));
    service.paste_content(&"hello".to_owned());

    service.poll_focus();
    service.poll_focus();
    assert!(
        !did(&service).contains(&Did::ScheduleExecute(POST_FOCUS_DELAY_MS)),
        "focus has not landed yet"
    );

    service.poll_focus();
    let did = did(&service);
    assert_eq!(did[did.len() - 2], Did::StopPolling);
    assert_eq!(
        did[did.len() - 1],
        Did::ScheduleExecute(POST_FOCUS_DELAY_MS)
    );
    assert!(service.has_pending_paste(), "still owed until it executes");
}

#[test]
fn polling_past_the_limit_drops_the_paste() {
    let mut service = PasteService::new(World::default().focus_never_lands());
    service.paste_content(&"hello".to_owned());

    for _ in 0..FOCUS_POLL_MAX {
        service.poll_focus();
    }

    assert!(!service.has_pending_paste(), "the paste is dropped");
    let did = did(&service);
    assert_eq!(
        did.last(),
        Some(&Did::StopPolling),
        "it stops polling and does not schedule anything: {did:?}"
    );
    assert!(!did.iter().any(|d| matches!(d, Did::Paste(_))));
}

#[test]
fn a_dropped_paste_leaves_the_content_on_the_clipboard() {
    // The timeout path never restores the clipboard, so what was copied is
    // still there. That is the whole consolation prize when focus never lands.
    let mut service = PasteService::new(World::default().focus_never_lands());
    service.paste_content(&"hello".to_owned());
    for _ in 0..FOCUS_POLL_MAX {
        service.poll_focus();
    }
    assert!(!did(&service).contains(&Did::RestoreClipboard));
}

#[test]
fn one_poll_short_of_the_limit_is_still_waiting() {
    let mut service = PasteService::new(World::default().focus_never_lands());
    service.paste_content(&"hello".to_owned());

    for _ in 0..FOCUS_POLL_MAX - 1 {
        service.poll_focus();
    }
    assert!(
        service.has_pending_paste(),
        "the limit is inclusive of the last try, not one before it"
    );
}

#[test]
fn executing_pastes_to_the_focused_window_and_its_app() {
    let mut service = PasteService::new(World {
        focused: Some(a_window()),
        app: Some(("org.editor.Editor".to_owned(), "editor.desktop".to_owned())),
        ..World::default()
    });
    service.paste_content(&"hello".to_owned());

    let target = service.execute_paste().expect("pasted");
    assert_eq!(target.window, Some(a_window()));
    assert_eq!(target.app.as_deref(), Some("editor.desktop"));
}

#[test]
fn a_window_whose_class_the_database_does_not_know_is_still_pasted_to() {
    let mut service = PasteService::new(World {
        focused: Some(a_window()),
        app: Some(("some.other.Class".to_owned(), "other.desktop".to_owned())),
        ..World::default()
    });
    service.paste_content(&"hello".to_owned());

    let target = service.execute_paste().expect("pasted");
    assert_eq!(target.app, None);
    assert!(
        target.window.is_some(),
        "an unknown app is not a reason not to paste"
    );
}

#[test]
fn a_paste_with_no_focused_window_at_all_still_happens() {
    // "Pasting to unknown window" — the C++ logs it and pastes anyway,
    // because the keystroke goes wherever focus is even if we cannot see it.
    let mut service = PasteService::new(World::default());
    service.paste_content(&"hello".to_owned());

    let target = service.execute_paste().expect("pasted");
    assert_eq!(target.window, None);
    assert_eq!(target.app, None);
    assert!(did(&service).iter().any(|d| matches!(d, Did::Paste(_))));
}

#[test]
fn the_clipboard_is_restored_after_a_paste() {
    let mut service = PasteService::new(World::default());
    service.paste_content(&"hello".to_owned());
    service.execute_paste();

    let did = did(&service);
    let paste = did.iter().position(|d| matches!(d, Did::Paste(_)));
    let restore = did.iter().position(|d| *d == Did::RestoreClipboard);
    assert!(
        paste < restore,
        "restore must come after the paste: {did:?}"
    );
}

#[test]
fn executing_twice_pastes_once() {
    // A late blind-paste timer arriving after a focus-driven paste would
    // otherwise paste the content a second time, into whatever has focus then.
    let mut service = PasteService::new(World::default());
    service.paste_content(&"hello".to_owned());

    assert!(service.execute_paste().is_some());
    assert_eq!(service.execute_paste(), None);
    assert_eq!(
        did(&service)
            .iter()
            .filter(|d| matches!(d, Did::Paste(_)))
            .count(),
        1
    );
}

#[test]
fn executing_with_nothing_pending_does_nothing_at_all() {
    let mut service = PasteService::new(World::default());
    assert_eq!(service.execute_paste(), None);
    assert!(did(&service).is_empty(), "not even a clipboard restore");
}

#[test]
fn a_second_paste_cancels_the_first_ones_poll() {
    let mut service = PasteService::new(World::default().focus_never_lands());
    service.paste_content(&"first".to_owned());
    service.poll_focus();
    service.poll_focus();
    service.paste_content(&"second".to_owned());

    // The poll count restarted, so the second paste gets the full five
    // seconds rather than what was left of the first one's.
    for _ in 0..FOCUS_POLL_MAX - 1 {
        service.poll_focus();
    }
    assert!(
        service.has_pending_paste(),
        "the second paste's budget must not be spent by the first's polls"
    );
}

#[test]
fn a_second_paste_copies_its_own_content() {
    let mut service = PasteService::new(World::default().focus_never_lands());
    service.paste_content(&"first".to_owned());
    service.paste_content(&"second".to_owned());

    let copies: Vec<_> = did(&service)
        .into_iter()
        .filter_map(|d| match d {
            Did::Copy(text) => Some(text),
            _ => None,
        })
        .collect();
    assert_eq!(copies, vec!["first".to_owned(), "second".to_owned()]);
}
