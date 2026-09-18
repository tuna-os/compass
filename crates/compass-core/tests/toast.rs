//! The toast queue, read against
//! `src/server/src/services/toast/toast-service.hpp`.

use compass_core::toast::{DEFAULT_DURATION_MS, Event, Toast, ToastService, ToastStyle};

fn toast(title: &str, style: ToastStyle, message: &str) -> Toast {
    Toast {
        title: title.to_owned(),
        message: message.to_owned(),
        style,
    }
}

#[test]
fn showing_a_toast_hides_the_old_one_first() {
    // `setToast` calls `clear()` before pushing, and `clear()` emits
    // `toastHidden` unconditionally -- so a single call raises two events, even
    // on an empty queue. A view that redraws on each one sees a blink; that is
    // what the C++ does today.
    let mut toasts = ToastService::new();
    toasts.success("Copied", "", 0);

    assert_eq!(
        toasts.drain_events(),
        [
            Event::Hidden,
            Event::Activated(toast("Copied", ToastStyle::Success, "")),
        ]
    );
    assert_eq!(
        toasts.current(),
        Some(&toast("Copied", ToastStyle::Success, ""))
    );
}

#[test]
fn failure_is_danger_not_a_style_of_its_own() {
    // `failure(...)` is `setToast(title, ToastStyle::Danger, message)`.
    let mut toasts = ToastService::new();
    toasts.failure("Could not copy", "the clipboard refused", 0);

    assert_eq!(toasts.current().map(|t| t.style), Some(ToastStyle::Danger));
    assert_eq!(
        toasts.current().map(|t| t.message.as_str()),
        Some("the clipboard refused")
    );
}

#[test]
fn a_toast_expires_after_two_seconds_by_default() {
    // `QTimer::singleShot(duration, ...)` with `int duration = 2000`.
    let mut toasts = ToastService::new();
    toasts.success("Copied", "", 0);
    let _ = toasts.drain_events();

    toasts.tick(DEFAULT_DURATION_MS - 1);
    assert!(toasts.current().is_some(), "not yet");
    assert!(toasts.drain_events().is_empty());

    toasts.tick(DEFAULT_DURATION_MS);
    assert!(toasts.current().is_none());
    assert_eq!(toasts.drain_events(), [Event::Hidden]);
    assert_eq!(DEFAULT_DURATION_MS, 2000);
}

#[test]
fn a_dynamic_toast_never_expires() {
    // `if (priority != ToastStyle::Dynamic)` guards the timer: a progress
    // toast stays until the command that raised it says otherwise.
    let mut toasts = ToastService::new();
    toasts.dynamic("Indexing", "0%", 0);
    let _ = toasts.drain_events();

    toasts.tick(DEFAULT_DURATION_MS * 100);
    assert_eq!(
        toasts.current(),
        Some(&toast("Indexing", ToastStyle::Dynamic, "0%"))
    );
    assert!(toasts.drain_events().is_empty());
}

#[test]
fn updating_a_dynamic_toast_changes_it_in_place() {
    // The `dynamic` fast path: `current->setTitle(...)` and `setMessage(...)`
    // rather than a replacement, so there is no `Hidden` in between.
    let mut toasts = ToastService::new();
    toasts.dynamic("Indexing", "0%", 0);
    let _ = toasts.drain_events();

    toasts.dynamic("Indexing", "50%", 500);
    assert_eq!(
        toasts.drain_events(),
        [Event::Activated(toast(
            "Indexing",
            ToastStyle::Dynamic,
            "50%"
        ))],
        "no Hidden: the toast was updated, not replaced"
    );
}

#[test]
fn dynamic_over_a_non_dynamic_toast_replaces_it() {
    // The guard is `current && current->priority() == ToastStyle::Dynamic`.
    let mut toasts = ToastService::new();
    toasts.success("Copied", "", 0);
    let _ = toasts.drain_events();

    toasts.dynamic("Indexing", "0%", 100);
    assert_eq!(
        toasts.drain_events(),
        [
            Event::Hidden,
            Event::Activated(toast("Indexing", ToastStyle::Dynamic, "0%")),
        ]
    );
}

#[test]
fn replacing_a_toast_restarts_the_clock() {
    // The C++ arms a timer per toast, so the replacement's two seconds start
    // when *it* was shown. The first toast's timer still fires at its own time
    // and finds nothing, which is why the second one is not cut short.
    let mut toasts = ToastService::new();
    toasts.success("First", "", 0);
    toasts.success("Second", "", 1_500);
    let _ = toasts.drain_events();

    // Where the first toast's two seconds would have been up.
    toasts.tick(2_000);
    assert_eq!(
        toasts.current(),
        Some(&toast("Second", ToastStyle::Success, "")),
        "the replacement keeps its own deadline"
    );
    assert!(toasts.drain_events().is_empty());

    toasts.tick(3_500);
    assert!(toasts.current().is_none());
    assert_eq!(toasts.drain_events(), [Event::Hidden]);
}

#[test]
fn clearing_hides_whatever_is_showing_and_says_so_even_when_empty() {
    let mut toasts = ToastService::new();
    toasts.clear();
    assert_eq!(toasts.drain_events(), [Event::Hidden]);

    toasts.success("Copied", "", 0);
    let _ = toasts.drain_events();
    toasts.clear();
    assert!(toasts.current().is_none());
    assert_eq!(toasts.drain_events(), [Event::Hidden]);
}

#[test]
fn an_expired_toast_is_hidden_once_not_on_every_tick() {
    // `destroyToast` erases the toast and only then calls `updateCurrent`, so
    // the hide happens once. A host that polls this every frame must not get a
    // `Hidden` per frame -- it would redraw for ever on an idle launcher.
    let mut toasts = ToastService::new();
    toasts.success("Copied", "", 0);
    let _ = toasts.drain_events();

    toasts.tick(2_000);
    assert_eq!(toasts.drain_events(), [Event::Hidden]);

    for now in [2_001, 2_500, 10_000] {
        toasts.tick(now);
        assert!(toasts.drain_events().is_empty(), "at {now}");
    }
}

#[test]
fn a_cleared_toast_does_not_come_back_when_its_timer_comes_due() {
    let mut toasts = ToastService::new();
    toasts.success("Copied", "", 0);
    toasts.clear();
    let _ = toasts.drain_events();

    toasts.tick(10_000);
    assert!(toasts.current().is_none());
    assert!(toasts.drain_events().is_empty(), "nothing left to hide");
}

#[test]
fn a_custom_duration_is_honoured() {
    let mut toasts = ToastService::new();
    toasts.set_toast("Slow", ToastStyle::Info, "", 10_000, 0);
    let _ = toasts.drain_events();

    toasts.tick(9_999);
    assert!(toasts.current().is_some());
    toasts.tick(10_000);
    assert!(toasts.current().is_none());
}
