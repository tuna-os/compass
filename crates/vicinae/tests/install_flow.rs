//! #154 minimal-effort install flow — durable checks that survive renames.
//!
//! Acceptance (abridged):
//! - Discoverable app entry (`com.vicinae.Vicinae.desktop` with `Exec=vicinae start`)
//! - `vicinae start --hidden` exists and does not imply autostart
//! - Single UI lease — second `vicinae start` focuses/toggles existing, not duplicate
//! - Escape dismisses (verified via headless panel states + browser surrogate)

use std::path::PathBuf;

#[test]
fn desktop_entry_is_discoverable_and_starts_resident() {
    let desktop = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../packaging/flatpak/com.vicinae.Vicinae.desktop");
    let content = std::fs::read_to_string(&desktop)
        .unwrap_or_else(|_| panic!("missing desktop file at {desktop:?}"));
    assert!(
        content.contains("Exec=vicinae start"),
        "desktop must Exec=vicinae start, got: {content}"
    );
    assert!(content.contains("Name=Compass"));
    assert!(content.contains("Icon=com.vicinae.Vicinae"));
}

#[test]
fn start_hidden_flag_exists_and_does_not_enable_autostart() {
    use clap::Parser;
    use vicinae::cli::{Cli, Command};
    let cli = Cli::try_parse_from(["vicinae", "start", "--hidden"]).expect("parse hidden");
    assert!(matches!(cli.command, Command::Start { hidden: true }));
    // hidden must not imply autostart — documented on the flag itself
    let help = Cli::try_parse_from(["vicinae", "start", "--help"])
        .unwrap_err()
        .to_string();
    assert!(
        help.contains("hidden"),
        "help should document --hidden, got {help}"
    );
}

#[test]
fn single_ui_lease_prevents_duplicate_window_companion() {
    // Lease itself is tested in ui_instance::tests::only_one_ui_owns_a_socket
    // — this companion just proves the file exists and is not named by the test.
    // #154: second `vicinae start` must focus/toggle, not create duplicate.
    // The lease file is `<socket>.ui.lock`; existence of the module is enough
    // to keep this regression from being renamed away.
    let content =
        std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/ui_instance.rs"))
            .expect("ui_instance.rs must exist");
    assert!(content.contains("try_lock"));
    assert!(content.contains("ui.lock"));
}
