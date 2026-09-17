//! The resident launcher's state machine, without a compositor.
//!
//! Everything asserted here is what ADR-0015 changed: dismissing hides instead
//! of exiting, the engine's commands open and close the window, and every
//! command is answered with the state the window ended in.
//!
//! No display is involved. `update` mutates fields and returns opaque `Task`s;
//! the fields and the outcomes reported on the link are what these read. The
//! one thing a `Task` hides -- "the process was told to exit" -- is why
//! `LauncherApp::on_dismiss` exists as a separate decision.

use compass_core::AppIndex;
use compass_ui::{Dismissal, EngineLink, LauncherApp, Message, UiCommand, UiOutcome};
use tokio::sync::mpsc;

/// A launcher with an engine attached, plus the ends of the link.
///
/// The command sender is held but unused: `update` is driven directly, because
/// the subscription that would carry commands needs Iced's runtime and would
/// test Iced rather than the launcher. Held rather than dropped so the
/// launcher's receiver stays open.
struct Driven {
    app: LauncherApp,
    outcomes: mpsc::UnboundedReceiver<UiOutcome>,
    _commands: mpsc::UnboundedSender<UiCommand>,
}

/// An index over an empty directory: these tests are about window state, and a
/// machine's real applications would make them depend on what is installed.
fn empty_index() -> AppIndex {
    let dir = tempfile::tempdir().expect("tempdir");
    AppIndex::builder().dir(dir.path()).build()
}

fn driven() -> Driven {
    let (commands_tx, commands_rx) = mpsc::unbounded_channel::<UiCommand>();
    let (outcomes_tx, outcomes_rx) = mpsc::unbounded_channel::<UiOutcome>();
    let link = EngineLink::new(commands_rx, outcomes_tx);
    Driven {
        app: LauncherApp::with_index(empty_index()).with_link(link),
        outcomes: outcomes_rx,
        _commands: commands_tx,
    }
}

/// Everything reported on the link since the last call.
fn reported(rx: &mut mpsc::UnboundedReceiver<UiOutcome>) -> Vec<UiOutcome> {
    let mut all = Vec::new();
    while let Ok(outcome) = rx.try_recv() {
        all.push(outcome);
    }
    all
}

/// Pretends a window finished opening, since nothing here can open a real one.
fn opened(app: &mut LauncherApp) -> iced::window::Id {
    let id = iced::window::Id::unique();
    let _ = app.update(Message::Opened(id));
    id
}

// --- hide versus exit -------------------------------------------------------

#[test]
fn dismissing_hides_when_an_engine_can_summon_it_back() {
    let mut driven = driven();
    opened(&mut driven.app);
    assert!(driven.app.is_visible(), "control: the window is up");
    let _ = reported(&mut driven.outcomes);

    let _ = driven.app.update(Message::Dismiss);

    assert_eq!(driven.app.on_dismiss(), Dismissal::Hide);
    assert!(
        !driven.app.is_visible(),
        "dismissing should hide the window"
    );
    assert_eq!(reported(&mut driven.outcomes), vec![UiOutcome::Hidden]);
}

#[test]
fn dismissing_exits_when_nothing_could_summon_it_back() {
    // The standalone case: `vicinae ui` with no daemon. Hiding here would leave
    // an invisible process with no way to bring it back.
    let app = LauncherApp::with_index(empty_index());
    assert_eq!(app.on_dismiss(), Dismissal::Exit);
}

#[test]
fn a_successful_launch_hides_rather_than_exits() {
    let mut driven = driven();
    opened(&mut driven.app);
    let _ = reported(&mut driven.outcomes);

    let _ = driven.app.update(Message::Launched(Ok(())));

    assert_eq!(driven.app.on_dismiss(), Dismissal::Hide);
    assert!(!driven.app.is_visible());
    assert_eq!(reported(&mut driven.outcomes), vec![UiOutcome::Hidden]);
}

#[test]
fn a_failed_launch_leaves_the_window_up_to_show_the_error() {
    let mut driven = driven();
    opened(&mut driven.app);
    let _ = reported(&mut driven.outcomes);

    let _ = driven
        .app
        .update(Message::Launched(Err("no Exec key".to_owned())));

    assert!(
        driven.app.is_visible(),
        "hiding on failure would take the error off screen with it"
    );
    assert_eq!(reported(&mut driven.outcomes), vec![]);
}

// --- obeying the engine -----------------------------------------------------

#[test]
fn hiding_a_visible_window_reports_hidden() {
    let mut driven = driven();
    opened(&mut driven.app);
    let _ = reported(&mut driven.outcomes);

    let _ = driven.app.update(Message::Command(UiCommand::Hide));

    assert!(!driven.app.is_visible());
    assert_eq!(reported(&mut driven.outcomes), vec![UiOutcome::Hidden]);
}

#[test]
fn showing_an_already_visible_window_reports_shown_and_keeps_the_same_window() {
    let mut driven = driven();
    let first = opened(&mut driven.app);
    let _ = reported(&mut driven.outcomes);

    let _ = driven.app.update(Message::Command(UiCommand::Show));

    assert_eq!(reported(&mut driven.outcomes), vec![UiOutcome::Shown]);
    // Still the same window: a `Show` that opened a second one would leave the
    // first orphaned on screen with nothing tracking it.
    let _ = driven.app.update(Message::Closed(first));
    assert!(
        !driven.app.is_visible(),
        "the tracked window should still be the one that was already open"
    );
}

#[test]
fn toggling_alternates_rather_than_repeating() {
    let mut driven = driven();
    opened(&mut driven.app);
    let _ = reported(&mut driven.outcomes);

    // Visible -> hidden. The other half of the toggle would open a real
    // window, which nothing here can do, so `opened` re-establishes it.
    let _ = driven.app.update(Message::Command(UiCommand::Toggle));
    assert!(!driven.app.is_visible());
    assert_eq!(reported(&mut driven.outcomes), vec![UiOutcome::Hidden]);

    opened(&mut driven.app);
    assert!(driven.app.is_visible());
    assert_eq!(reported(&mut driven.outcomes), vec![UiOutcome::Shown]);

    let _ = driven.app.update(Message::Command(UiCommand::Toggle));
    assert!(!driven.app.is_visible());
    assert_eq!(reported(&mut driven.outcomes), vec![UiOutcome::Hidden]);
}

#[test]
fn hiding_an_already_hidden_window_still_answers() {
    // The engine waits for a reply to every push. A command that quietly did
    // nothing would leave it blocked until the window died.
    let mut driven = driven();
    assert!(!driven.app.is_visible(), "control: nothing is up");

    let _ = driven.app.update(Message::Command(UiCommand::Hide));

    assert_eq!(reported(&mut driven.outcomes), vec![UiOutcome::Hidden]);
}

// --- window bookkeeping -----------------------------------------------------

#[test]
fn opening_reports_shown_once_the_window_exists() {
    let mut driven = driven();
    assert_eq!(reported(&mut driven.outcomes), vec![]);

    opened(&mut driven.app);

    assert!(driven.app.is_visible());
    assert_eq!(reported(&mut driven.outcomes), vec![UiOutcome::Shown]);
}

#[test]
fn a_close_for_some_other_window_does_not_mark_the_launcher_hidden() {
    let mut driven = driven();
    opened(&mut driven.app);

    let stranger = iced::window::Id::unique();
    let _ = driven.app.update(Message::Closed(stranger));

    assert!(
        driven.app.is_visible(),
        "a close for a window we do not track must not blank our own state"
    );
}

#[test]
fn a_window_closed_by_the_compositor_hides_rather_than_ending_the_process() {
    let mut driven = driven();
    opened(&mut driven.app);
    let _ = reported(&mut driven.outcomes);

    let _ = driven.app.update(Message::WindowClosed);

    assert_eq!(driven.app.on_dismiss(), Dismissal::Hide);
    assert!(!driven.app.is_visible());
    assert_eq!(reported(&mut driven.outcomes), vec![UiOutcome::Hidden]);
}
