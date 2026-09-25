//! The Power Management builtin, read against
//! `src/server/src/builtins/power-management/power-management-extension.cpp`.

use compass_core::power_commands::{
    COMMANDS, CONFIRM_BODY, CONFIRM_PREFERENCE, CONFIRM_TITLE, CUSTOM_PROGRAM_DESCRIPTION,
    CUSTOM_PROGRAM_PREFERENCE, CUSTOM_PROGRAM_TITLE, EXTENSION_DESCRIPTION, EXTENSION_ID,
    EXTENSION_NAME, Preference, Step, command, custom_program_failure, plan, preferences,
};

fn get(id: &str) -> &'static compass_core::power_commands::PowerCommand {
    command(id).unwrap_or_else(|| panic!("{id} is a power command"))
}

#[test]
fn the_catalogue_is_the_eight_commands_in_registration_order() {
    // `PowerManagementExtension::PowerManagementExtension()` registers five
    // unconditionally and three more on Linux.
    let ids: Vec<&str> = COMMANDS.iter().map(|command| command.id).collect();
    assert_eq!(
        ids,
        [
            "power-off",
            "reboot",
            "sleep",
            "lock",
            "logout",
            "suspend",
            "hibernate",
            "soft-reboot",
        ]
    );
    assert_eq!(EXTENSION_ID, "power");
    assert_eq!(EXTENSION_NAME, "Power Management");
    assert_eq!(
        EXTENSION_DESCRIPTION,
        "Power off, suspend, sleep, hibernate your computer."
    );
}

#[test]
fn only_locking_skips_the_confirmation_by_default() {
    // `requiresDefaultConfirmation()` is true on the base class; only
    // `LockCommand` overrides it, because locking is not destructive.
    for command in COMMANDS {
        assert_eq!(
            command.confirm_by_default,
            command.id != "lock",
            "{}",
            command.id
        );
    }
}

#[test]
fn the_descriptions_are_the_ones_the_user_reads() {
    // Long strings, read out of the source rather than paraphrased: the
    // hibernate and suspend ones exist to tell the user which is which.
    assert_eq!(
        get("hibernate").description,
        "Suspend the system to disk. This turns off the system completely and saves its state on disk, to be restored on next boot."
    );
    assert_eq!(
        get("suspend").description,
        "Suspend the system to RAM. Unlike hibernation, this does not turn the computer off and will break on power loss."
    );
    assert_eq!(
        get("soft-reboot").description,
        "Soft reboot the system, which usually means only userspace is rebooted."
    );
    assert_eq!(
        get("logout").description,
        "Terminate the current user session. If you simply want to lock your session you should use 'Lock Session' instead."
    );
    assert_eq!(get("power-off").description, "Power off the system");
}

#[test]
fn the_keywords_are_what_the_user_is_likely_to_type() {
    assert_eq!(get("power-off").keywords, ["shutdown"]);
    assert_eq!(get("reboot").keywords, ["restart"]);
    assert_eq!(get("soft-reboot").keywords, ["restart"]);
    assert_eq!(get("hibernate").keywords, ["disk", "suspend"]);
    assert_eq!(get("suspend").keywords, ["suspend"]);
    assert_eq!(get("lock").keywords, ["lock"]);
    assert_eq!(get("logout").keywords, ["logout"]);
    assert!(
        get("sleep").keywords.is_empty(),
        "sleep declares none, so only its title matches"
    );
}

#[test]
fn the_failure_messages_are_inconsistent_and_stay_that_way() {
    // "cannot" for power off and suspend, "can't" for the other six. Copying
    // the inconsistency is the point: these strings are translated, and
    // normalising them here would orphan the translations.
    let cannot: Vec<&str> = COMMANDS
        .iter()
        .filter(|command| command.cannot_message.contains("cannot"))
        .map(|command| command.id)
        .collect();
    assert_eq!(cannot, ["power-off", "suspend"]);

    assert_eq!(get("power-off").cannot_message, "System cannot power off");
    assert_eq!(get("suspend").cannot_message, "System cannot suspend");
    assert_eq!(get("hibernate").cannot_message, "System can't hibernate");
    assert_eq!(get("logout").cannot_message, "System can't logout");
    assert_eq!(get("logout").failed_message, "Failed to log out");
    assert_eq!(
        get("soft-reboot").cannot_message,
        "System can't soft reboot"
    );
}

#[test]
fn every_command_declares_the_confirmation_preference() {
    for command in COMMANDS {
        let prefs = preferences(command, false);
        assert_eq!(
            prefs,
            [Preference::Checkbox {
                name: CONFIRM_PREFERENCE,
                title: "Ask for confirmation",
                default: command.confirm_by_default,
            }],
            "{}",
            command.id
        );
    }
}

#[test]
fn the_custom_program_preference_exists_only_where_a_shell_makes_sense() {
    // `supportsCustomProgram()` is `false` on macOS and Windows.
    let prefs = preferences(get("reboot"), true);
    assert_eq!(prefs.len(), 2);
    assert_eq!(
        prefs[1],
        Preference::Text {
            name: CUSTOM_PROGRAM_PREFERENCE,
            title: CUSTOM_PROGRAM_TITLE,
            description: CUSTOM_PROGRAM_DESCRIPTION,
            required: false,
        }
    );
    assert_eq!(preferences(get("reboot"), false).len(), 1);
}

#[test]
fn a_command_that_asks_first_puts_the_dialog_before_everything() {
    let steps = plan(get("reboot"), true, None);

    assert_eq!(
        steps[0],
        Step::Confirm {
            title: CONFIRM_TITLE,
            body: CONFIRM_BODY,
        }
    );
    assert_eq!(CONFIRM_TITLE, "Are you sure");
    assert_eq!(CONFIRM_BODY, "High-impact operation, please confirm");
    assert!(steps.contains(&Step::Perform { id: "reboot" }));
}

#[test]
fn a_command_that_does_not_ask_goes_straight_to_closing_and_doing() {
    let steps = plan(get("lock"), false, None);

    assert_eq!(
        steps,
        [
            Step::CloseWindow,
            Step::Perform { id: "lock" },
            Step::CloseWindow,
        ]
    );
}

#[test]
fn the_window_is_closed_twice_because_the_cpp_closes_it_twice() {
    // The alert's callback closes it, then the shared handler closes it again
    // at the end. Closing a closed window is a no-op; tidying it away here
    // would be guessing that nothing depends on the second call.
    let steps = plan(get("power-off"), true, None);
    assert_eq!(
        steps
            .iter()
            .filter(|step| **step == Step::CloseWindow)
            .count(),
        2
    );
}

#[test]
fn a_custom_program_replaces_the_built_in_behaviour() {
    let steps = plan(get("power-off"), false, Some("systemctl poweroff"));

    assert_eq!(
        steps,
        [
            Step::CloseWindow,
            Step::RunCustomProgram {
                program: "systemctl poweroff".to_owned(),
            },
            Step::CloseWindow,
        ]
    );
    assert!(
        !steps
            .iter()
            .any(|step| matches!(step, Step::Perform { .. })),
        "the logind call does not also happen"
    );
}

#[test]
fn an_empty_custom_program_is_no_program_at_all() {
    // `if (auto prog = ...; !prog.isEmpty())` -- an empty preference means the
    // default behaviour, not a command called "".
    let steps = plan(get("power-off"), false, Some(""));
    assert!(steps.contains(&Step::Perform { id: "power-off" }));
    assert!(
        !steps
            .iter()
            .any(|step| matches!(step, Step::RunCustomProgram { .. }))
    );
}

#[test]
fn a_failing_custom_program_names_itself_in_the_toast() {
    assert_eq!(
        custom_program_failure("systemctl poweroff"),
        "Failed to execute custom program systemctl poweroff"
    );
}

fn stored(json: &str) -> serde_json::Map<String, serde_json::Value> {
    serde_json::from_str(json).expect("a preferences object")
}

#[test]
fn the_confirm_preference_overrides_the_default_either_way() {
    use compass_core::power_commands::should_confirm;
    let (reboot, lock) = (get("reboot"), get("lock"));
    assert!(should_confirm(reboot, None));
    assert!(!should_confirm(lock, None));
    assert!(!should_confirm(
        reboot,
        Some(&stored(r#"{"confirm": false}"#))
    ));
    assert!(should_confirm(lock, Some(&stored(r#"{"confirm": true}"#))));
    assert!(
        should_confirm(reboot, Some(&stored(r#"{"confirm": "no"}"#))),
        "a confirm that is not a boolean is not a setting"
    );
}

#[test]
fn the_custom_program_is_read_and_an_empty_one_is_none() {
    use compass_core::power_commands::custom_program;
    let set = stored(r#"{"customProgram": "systemctl poweroff"}"#);
    assert_eq!(custom_program(Some(&set)), Some("systemctl poweroff"));
    assert_eq!(
        custom_program(Some(&stored(r#"{"customProgram": ""}"#))),
        None
    );
    assert_eq!(custom_program(None), None);
}
