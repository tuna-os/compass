//! Small semantic `insta` snapshots of what `view()` actually lays out.
//!
//! # What this replaces
//!
//! The first version of this file claimed, in its own doc comment, to render
//! "via `iced_test::Simulator`". It did not. It built a string from a
//! hand-written view description and snapshotted that, and its control
//! compared two different hand-written strings — which proves `format!` is
//! injective, not that any widget exists. Deleting the whole of `view()`
//! would have left all five tests green.
//!
//! # Structure, not pixels, and not strings
//!
//! `Simulator::snapshot` compares a committed PNG, which makes the assertion
//! depend on the fonts the runner happens to ship; `app.rs` already rejects
//! that for the same reason. So the snapshot here is the **layout order** of
//! the labels the view puts on screen, read back out of the real Iced tree by
//! selecting each one and taking its bounds. A row that disappears, a section
//! that moves above another, or a branch of `view()` that stops rendering all
//! change it. A font package that does not, does not.
//!
//! # Deliberately three, not four
//!
//! PLAN.md §8.5 names four: results list, empty state, detail view, form.
//! The Rust UI has no detail view and no form — there is no such branch in
//! `view()` to render. Snapshotting a description of one would be the same
//! fiction this file just removed, so they are left out and the gap is stated
//! here instead.

use compass_core::apps::AppIndex;
use compass_ui::LauncherApp;
use compass_ui::message::Message;
use insta::assert_snapshot;

/// Labels probed in every snapshot, so an unexpected appearance is as visible
/// as a disappearance.
const PROBES: &[&str] = &[
    "Search…",
    "Application 00",
    "Application 01",
    "Application 02",
    "No results",
    "Open",
    "Copy name",
    "COPY",
];

fn app_with(dir: &std::path::Path, count: usize) -> LauncherApp {
    for i in 0..count {
        std::fs::write(
            dir.join(format!("app-{i:02}.desktop")),
            format!("[Desktop Entry]\nType=Application\nName=Application {i:02}\nExec=/bin/true\n"),
        )
        .expect("write a desktop entry");
    }
    LauncherApp::with_index(AppIndex::builder().dir(dir).build())
}

/// Renders `view()` headlessly and reports which probes are on screen, in
/// layout order: top to bottom, then left to right.
fn layout(app: &LauncherApp) -> String {
    let mut ui = iced_test::Simulator::with_size(
        iced::Settings::default(),
        iced::Size::new(800.0, 480.0),
        app.view(),
    );

    let mut found: Vec<(i32, i32, &str)> = Vec::with_capacity(PROBES.len());
    let mut absent: Vec<&str> = Vec::with_capacity(PROBES.len());

    for probe in PROBES {
        match ui.find(*probe) {
            Ok(target) => {
                let bounds = target.bounds();
                // Rounded to whole pixels: sub-pixel layout differs between
                // font stacks and is not what this asserts.
                found.push((bounds.y.round() as i32, bounds.x.round() as i32, probe));
            }
            Err(_) => absent.push(probe),
        }
    }

    found.sort_unstable();

    let mut out = String::new();
    out.push_str("on screen, in layout order:\n");
    for (_, _, probe) in &found {
        out.push_str("  ");
        out.push_str(probe);
        out.push('\n');
    }
    out.push_str("not rendered:\n");
    for probe in &absent {
        out.push_str("  ");
        out.push_str(probe);
        out.push('\n');
    }
    out
}

#[test]
fn results_list() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut app = app_with(dir.path(), 3);
    let _ = app.update(Message::QueryChanged("Application".to_owned()));
    assert_snapshot!("results_list", layout(&app));
}

#[test]
fn empty_state() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut app = app_with(dir.path(), 3);
    let _ = app.update(Message::QueryChanged("zzzzzzzz".to_owned()));
    assert_snapshot!("empty_state", layout(&app));
}

#[test]
fn action_panel() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut app = app_with(dir.path(), 3);
    let _ = app.update(Message::QueryChanged("Application".to_owned()));
    let _ = app.update(Message::TogglePanel);
    assert_snapshot!("action_panel", layout(&app));
}

/// The labels the snapshot says are on screen, as opposed to the ones it
/// lists as absent.
///
/// A plain `contains` over the whole snapshot cannot tell those apart — every
/// probe appears in it either way. The first version of the control below did
/// exactly that and passed while `view()` was mutated out from under it.
fn on_screen(snapshot: &str) -> Vec<&str> {
    snapshot
        .lines()
        .skip_while(|line| *line != "on screen, in layout order:")
        .skip(1)
        .take_while(|line| *line != "not rendered:")
        .map(str::trim)
        .collect()
}

/// CONTROL. The snapshots above are only worth keeping if they move when the
/// rendered tree moves. Each pair differs in the state fed to the same
/// `view()`, so an equal result means the harness is reading nothing.
#[test]
fn the_snapshots_move_when_the_rendered_tree_moves() {
    let dir = tempfile::tempdir().expect("tempdir");

    let mut listed = app_with(dir.path(), 3);
    let _ = listed.update(Message::QueryChanged("Application".to_owned()));
    let listed = layout(&listed);

    let mut empty = app_with(dir.path(), 3);
    let _ = empty.update(Message::QueryChanged("zzzzzzzz".to_owned()));
    let empty = layout(&empty);

    let mut panelled = app_with(dir.path(), 3);
    let _ = panelled.update(Message::QueryChanged("Application".to_owned()));
    let _ = panelled.update(Message::TogglePanel);
    let panelled = layout(&panelled);

    assert_ne!(listed, empty, "an empty query must not lay out like a list");
    assert_ne!(listed, panelled, "opening the panel must change the layout");
    assert!(
        on_screen(&listed).contains(&"Application 00"),
        "the control is vacuous if the list never rendered a row: {listed}"
    );
    assert!(
        on_screen(&empty).contains(&"No results"),
        "the control is vacuous if the empty state never rendered: {empty}"
    );
}
