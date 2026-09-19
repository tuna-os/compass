//! Who is holding the keyboard, and what is blurred behind what.
//!
//! Read off `WaylandShortcutInhibitManager`
//! (`src/server/src/services/shortcut-inhibit/`) and
//! `ExtBackgroundEffectV1Manager` (`src/server/src/services/window-material/`).

use compass_core::window_effects::{
    Applied, MaterialParams, Rect, ShortcutInhibitManager, WindowMaterialManager,
};

/// Two distinct windows.
const LAUNCHER: u64 = 1;
const SETTINGS: u64 = 2;

/// A blur of `radius` over the whole of a small rectangle.
fn params(radius: i32) -> MaterialParams {
    MaterialParams {
        radius,
        region: Rect {
            x: 0,
            y: 0,
            width: 800,
            height: 600,
        },
    }
}

#[test]
fn a_compositor_without_the_protocol_inhibits_nothing() {
    let mut manager = ShortcutInhibitManager::new(false);
    assert!(!manager.is_supported());
    assert!(!manager.inhibit(LAUNCHER));
    assert_eq!(manager.inhibitor_count(), 0);
}

#[test]
fn inhibiting_takes_the_keyboard_for_that_window() {
    let mut manager = ShortcutInhibitManager::new(true);
    assert!(manager.inhibit(LAUNCHER));
    assert!(manager.is_inhibiting(LAUNCHER));
    assert!(!manager.is_inhibiting(SETTINGS));
}

#[test]
fn inhibiting_twice_creates_one_inhibitor() {
    // Two inhibitors for one window is a leak, and the one left over keeps the
    // keyboard grabbed after the window is gone.
    let mut manager = ShortcutInhibitManager::new(true);
    assert!(manager.inhibit(LAUNCHER));
    assert!(manager.inhibit(LAUNCHER));
    assert_eq!(manager.inhibitor_count(), 1);
}

#[test]
fn a_window_already_inhibiting_is_told_yes_even_if_support_went_away() {
    // The contains check comes before the support check, and this is the case
    // that distinguishes the two orders: a Wayland global can be withdrawn
    // while the process runs, and a manager answering false here would invite
    // the caller to stop tracking a grab it is still holding.
    let mut manager = ShortcutInhibitManager::new(true);
    manager.inhibit(LAUNCHER);

    manager.set_supported(false);
    assert!(
        manager.inhibit(LAUNCHER),
        "a window already holding the keyboard still reports held"
    );
    assert!(
        !manager.inhibit(SETTINGS),
        "but a fresh one cannot start once the protocol has gone"
    );
}

#[test]
fn releasing_gives_the_keyboard_back() {
    let mut manager = ShortcutInhibitManager::new(true);
    manager.inhibit(LAUNCHER);
    assert!(manager.release(LAUNCHER));
    assert!(!manager.is_inhibiting(LAUNCHER));
    assert_eq!(manager.inhibitor_count(), 0);
}

#[test]
fn releasing_something_never_inhibited_is_false_and_not_a_failure() {
    // Which is what lets a caller release unconditionally on the way out.
    let mut manager = ShortcutInhibitManager::new(true);
    assert!(!manager.release(LAUNCHER));
}

#[test]
fn releasing_one_window_leaves_another_holding() {
    let mut manager = ShortcutInhibitManager::new(true);
    manager.inhibit(LAUNCHER);
    manager.inhibit(SETTINGS);

    manager.release(LAUNCHER);
    assert!(!manager.is_inhibiting(LAUNCHER));
    assert!(manager.is_inhibiting(SETTINGS));
}

#[test]
fn a_destroyed_surface_gives_the_keyboard_back() {
    // A window can go away without anyone calling release, and an inhibitor
    // outliving its surface is how the keyboard stays grabbed with nothing
    // left to give it back. The C++ installs an event filter for this.
    let mut manager = ShortcutInhibitManager::new(true);
    manager.inhibit(LAUNCHER);

    manager.surface_destroyed(LAUNCHER);
    assert!(!manager.is_inhibiting(LAUNCHER));
    assert_eq!(manager.inhibitor_count(), 0);
}

#[test]
fn a_destroyed_surface_that_was_not_inhibiting_changes_nothing() {
    let mut manager = ShortcutInhibitManager::new(true);
    manager.inhibit(SETTINGS);
    manager.surface_destroyed(LAUNCHER);
    assert!(manager.is_inhibiting(SETTINGS));
}

#[test]
fn a_compositor_without_blur_applies_nothing() {
    let mut manager = WindowMaterialManager::new(false);
    assert_eq!(manager.apply(LAUNCHER, params(10)), Applied::Unsupported);
    assert!(!manager.apply(LAUNCHER, params(10)).succeeded());
    assert_eq!(manager.effect_count(), 0);
}

#[test]
fn applying_creates_an_effect() {
    let mut manager = WindowMaterialManager::new(true);
    assert_eq!(manager.apply(LAUNCHER, params(10)), Applied::Created);
    assert_eq!(manager.params(LAUNCHER), Some(params(10)));
}

#[test]
fn applying_the_same_parameters_again_sends_nothing() {
    // Re-uploading a blur region every frame is a visible stutter with no
    // error anywhere, which is exactly the kind of bug a value comparison
    // prevents and nothing else catches.
    let mut manager = WindowMaterialManager::new(true);
    manager.apply(LAUNCHER, params(10));

    assert_eq!(manager.apply(LAUNCHER, params(10)), Applied::Unchanged);
    assert!(manager.apply(LAUNCHER, params(10)).succeeded());
}

#[test]
fn applying_different_parameters_updates_the_effect() {
    let mut manager = WindowMaterialManager::new(true);
    manager.apply(LAUNCHER, params(10));

    assert_eq!(manager.apply(LAUNCHER, params(20)), Applied::Updated);
    assert_eq!(manager.params(LAUNCHER), Some(params(20)));
    assert_eq!(manager.effect_count(), 1, "updated, not added");
}

#[test]
fn a_changed_region_counts_as_different_parameters() {
    // Params compares both fields; comparing only the radius would leave the
    // blur behind when the window is resized.
    let mut manager = WindowMaterialManager::new(true);
    manager.apply(LAUNCHER, params(10));

    let resized = MaterialParams {
        radius: 10,
        region: Rect {
            x: 0,
            y: 0,
            width: 1000,
            height: 600,
        },
    };
    assert_eq!(manager.apply(LAUNCHER, resized), Applied::Updated);
}

#[test]
fn the_support_check_comes_before_the_registry_here() {
    // The opposite way round from the inhibitor, and that asymmetry is in the
    // C++: a stale blur is cosmetic, a stale keyboard grab locks a person out
    // of their shortcuts, so only the inhibitor keeps answering for a window
    // it already holds.
    let mut manager = WindowMaterialManager::new(true);
    manager.apply(LAUNCHER, params(10));

    manager.set_supported(false);
    assert_eq!(
        manager.apply(LAUNCHER, params(10)),
        Applied::Unsupported,
        "unlike the inhibitor, an existing effect does not keep answering"
    );
}

#[test]
fn each_window_keeps_its_own_effect() {
    let mut manager = WindowMaterialManager::new(true);
    manager.apply(LAUNCHER, params(10));
    manager.apply(SETTINGS, params(30));

    assert_eq!(manager.params(LAUNCHER), Some(params(10)));
    assert_eq!(manager.params(SETTINGS), Some(params(30)));
    assert_eq!(manager.effect_count(), 2);
}

#[test]
fn clearing_removes_the_effect_and_says_there_was_one() {
    let mut manager = WindowMaterialManager::new(true);
    manager.apply(LAUNCHER, params(10));

    assert!(manager.clear(LAUNCHER));
    assert_eq!(manager.params(LAUNCHER), None);
    assert!(!manager.clear(LAUNCHER), "and there is not one any more");
}

#[test]
fn a_destroyed_surface_drops_its_effect() {
    let mut manager = WindowMaterialManager::new(true);
    manager.apply(LAUNCHER, params(10));
    manager.apply(SETTINGS, params(10));

    manager.surface_destroyed(LAUNCHER);
    assert_eq!(manager.params(LAUNCHER), None);
    assert_eq!(manager.params(SETTINGS), Some(params(10)));
}

#[test]
fn an_effect_can_be_applied_again_after_it_is_cleared() {
    let mut manager = WindowMaterialManager::new(true);
    manager.apply(LAUNCHER, params(10));
    manager.clear(LAUNCHER);

    assert_eq!(
        manager.apply(LAUNCHER, params(10)),
        Applied::Created,
        "created again, not unchanged"
    );
}
