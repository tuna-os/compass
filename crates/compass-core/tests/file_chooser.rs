//! Which file dialog opens, and what closing it reports.
//!
//! Read off `FileChooserService` (`src/server/src/services/file-chooser/`).

use std::path::PathBuf;

use compass_core::file_chooser::{FileChooserOptions, FileChooserService, Opened, to_local_path};

/// A portal that is there.
fn portal_available() -> bool {
    true
}

/// A portal that is not.
fn no_portal() -> bool {
    false
}

#[test]
fn a_missing_portal_is_reported_as_not_handled_so_the_caller_shows_its_own() {
    // The one rule here worth breaking a build over. A caller that read false
    // as an error would show no dialog at all on every desktop without a
    // working portal.
    let mut service = FileChooserService::new();
    let opened = service.open_dialog(FileChooserOptions::new(), no_portal);

    assert_eq!(opened, Opened::Fallback);
    assert!(!opened.portal_handled());
    assert!(service.is_active(), "the fallback dialog is up");
}

#[test]
fn a_working_portal_handles_it() {
    let mut service = FileChooserService::new();
    let opened = service.open_dialog(FileChooserOptions::new(), portal_available);

    assert!(opened.portal_handled());
    assert_eq!(opened, Opened::Portal(FileChooserOptions::new()));
    assert!(service.is_active());
}

#[test]
fn the_options_reach_the_portal_unchanged() {
    let mut service = FileChooserService::new();
    let options = FileChooserOptions {
        can_choose_files: false,
        can_choose_directories: true,
        allow_multiple_selection: true,
        show_hidden_files: true,
        current_folder: Some(PathBuf::from("/home/ada")),
    };
    let opened = service.open_dialog(options.clone(), portal_available);
    assert_eq!(opened, Opened::Portal(options));
}

#[test]
fn the_default_options_are_one_file_no_directories_no_hidden() {
    let options = FileChooserOptions::new();
    assert!(options.can_choose_files);
    assert!(!options.can_choose_directories);
    assert!(!options.allow_multiple_selection);
    assert!(!options.show_hidden_files);
    assert_eq!(options.current_folder, None);
}

#[test]
fn a_second_open_while_one_is_up_does_nothing() {
    // Two dialogs at once is the thing the early return exists to prevent, and
    // it reports handled either way because from the caller's side there is a
    // dialog on screen.
    let mut service = FileChooserService::new();
    service.open_dialog(FileChooserOptions::new(), portal_available);

    let second = service.open_dialog(FileChooserOptions::new(), || {
        panic!("the portal must not be asked again")
    });
    assert_eq!(second, Opened::AlreadyOpen);
    assert!(second.portal_handled());
}

#[test]
fn a_second_open_while_the_fallback_is_up_does_nothing_either() {
    let mut service = FileChooserService::new();
    service.open_dialog(FileChooserOptions::new(), no_portal);

    let second = service.open_dialog(FileChooserOptions::new(), || {
        panic!("the portal must not be asked again")
    });
    assert_eq!(second, Opened::AlreadyOpen);
}

#[test]
fn a_selection_closes_the_dialog_and_reports_the_paths() {
    let mut service = FileChooserService::new();
    service.open_dialog(FileChooserOptions::new(), portal_available);

    let closed = service
        .finish(Some(vec![PathBuf::from("/tmp/a")]))
        .expect("closed");
    assert_eq!(closed.selected, Some(vec![PathBuf::from("/tmp/a")]));
    assert!(!service.is_active());
}

#[test]
fn an_accepted_dialog_with_no_files_is_not_a_cancellation() {
    // The portal can accept and return nothing. The C++ still emits
    // filesSelected, so a caller that treated the empty list as a cancel would
    // diverge on a case the portal really produces.
    let mut service = FileChooserService::new();
    service.open_dialog(FileChooserOptions::new(), portal_available);

    let closed = service.finish(Some(Vec::new())).expect("closed");
    assert_eq!(closed.selected, Some(Vec::new()));
    assert_ne!(closed.selected, None);
}

#[test]
fn cancelling_closes_without_a_selection() {
    let mut service = FileChooserService::new();
    service.open_dialog(FileChooserOptions::new(), portal_available);

    let closed = service.cancel().expect("closed");
    assert_eq!(closed.selected, None);
    assert!(!service.is_active());
}

#[test]
fn a_late_answer_after_the_dialog_closed_announces_nothing() {
    // Otherwise a portal reply arriving after a cancel would announce a
    // selection the person never made.
    let mut service = FileChooserService::new();
    service.open_dialog(FileChooserOptions::new(), portal_available);
    service.cancel();

    assert_eq!(service.finish(Some(vec![PathBuf::from("/tmp/a")])), None);
}

#[test]
fn finishing_with_nothing_open_announces_nothing() {
    let mut service = FileChooserService::new();
    assert_eq!(service.cancel(), None);
    assert_eq!(service.finish(Some(Vec::new())), None);
}

#[test]
fn the_fallback_reports_its_own_completion_and_carries_no_selection() {
    // notifyFallbackDone only says the dialog is gone; the fallback reports
    // what was picked through its own channel.
    let mut service = FileChooserService::new();
    service.open_dialog(FileChooserOptions::new(), no_portal);

    let closed = service.notify_fallback_done().expect("closed");
    assert_eq!(closed.selected, None);
    assert!(!service.is_active());
}

#[test]
fn the_fallback_completion_does_not_close_a_portal_dialog() {
    // They are different dialogs on different paths; letting one close the
    // other would leave the portal's answer arriving to nothing.
    let mut service = FileChooserService::new();
    service.open_dialog(FileChooserOptions::new(), portal_available);

    assert_eq!(service.notify_fallback_done(), None);
    assert!(service.is_active(), "the portal dialog is still up");
}

#[test]
fn a_dialog_can_be_opened_again_after_it_closes() {
    let mut service = FileChooserService::new();
    service.open_dialog(FileChooserOptions::new(), portal_available);
    service.cancel();

    let again = service.open_dialog(FileChooserOptions::new(), portal_available);
    assert_eq!(again, Opened::Portal(FileChooserOptions::new()));
}

#[test]
fn cancelling_also_clears_a_fallback_dialog() {
    let mut service = FileChooserService::new();
    service.open_dialog(FileChooserOptions::new(), no_portal);
    assert!(service.cancel().is_some());
    assert!(!service.is_active());
}

#[test]
fn a_trailing_separator_is_dropped_from_a_directory_url() {
    assert_eq!(to_local_path("/home/ada/Documents/"), "/home/ada/Documents");
}

#[test]
fn the_root_keeps_its_slash() {
    // Chopping it would give "", which names nothing.
    assert_eq!(to_local_path("/"), "/");
}

#[test]
fn a_windows_drive_root_keeps_its_slash() {
    // "C:" alone names the process's current directory on that drive, not the
    // drive itself, which is why the C++ tests for ":/" specifically.
    assert_eq!(to_local_path("C:/"), "C:/");
    assert_eq!(to_local_path("C:/Users/"), "C:/Users");
}

#[test]
fn a_path_with_no_trailing_separator_is_left_alone() {
    assert_eq!(to_local_path("/home/ada/notes.txt"), "/home/ada/notes.txt");
    assert_eq!(to_local_path(""), "");
}
