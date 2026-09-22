//! The Power Management builtin: eight commands, what each is called, and what
//! each does when it fails.
//!
//! A port of `power-management-extension.cpp`
//! (`src/server/src/builtins/power-management/`). The commands themselves are
//! one logind call each — `compass_power` makes those — so what is here is
//! everything around the call: the catalogue, the confirmation preference, the
//! custom-program escape hatch, and the two failure messages per command, whose
//! wording is not consistent and is reproduced as it is.

/// One power command, as the root list sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PowerCommand {
    /// The command id within the `power` extension.
    pub id: &'static str,
    /// Its title.
    pub name: &'static str,
    /// Its subtitle.
    pub description: &'static str,
    /// Extra search terms.
    pub keywords: &'static [&'static str],
    /// Whether the `confirm` preference defaults to on.
    ///
    /// `requiresDefaultConfirmation()` is true on the base class; only Lock
    /// overrides it to false, because locking is not destructive.
    pub confirm_by_default: bool,
    /// What the toast says when the system reports it cannot do this.
    pub cannot_message: &'static str,
    /// What the toast says when the attempt itself fails.
    pub failed_message: &'static str,
}

/// The commands, in the order `PowerManagementExtension` registers them on
/// Linux.
///
/// Registration order is the order they appear, and the last three are inside
/// `#elif !defined(Q_OS_MACOS)`: a macOS build has five commands, a Windows
/// build six (it takes Hibernate but not Suspend or Soft Reboot).
pub const COMMANDS: &[PowerCommand] = &[
    PowerCommand {
        id: "power-off",
        name: "Power Off System",
        description: "Power off the system",
        keywords: &["shutdown"],
        confirm_by_default: true,
        // "cannot", where most of the others say "can't".
        cannot_message: "System cannot power off",
        failed_message: "Failed to power off",
    },
    PowerCommand {
        id: "reboot",
        name: "Reboot System",
        description: "Reboot the system",
        keywords: &["restart"],
        confirm_by_default: true,
        cannot_message: "System can't reboot",
        failed_message: "Failed to reboot",
    },
    PowerCommand {
        id: "sleep",
        name: "Put System to Sleep",
        description: "Put system to sleep",
        keywords: &[],
        confirm_by_default: true,
        cannot_message: "System can't sleep",
        failed_message: "Failed to sleep",
    },
    PowerCommand {
        id: "lock",
        name: "Lock Session",
        description: "Lock the current user session",
        keywords: &["lock"],
        // The only one that does not ask first.
        confirm_by_default: false,
        cannot_message: "System can't lock",
        failed_message: "Failed to lock",
    },
    PowerCommand {
        id: "logout",
        name: "Log Out",
        description: "Terminate the current user session. If you simply want to lock your session you should use 'Lock Session' instead.",
        keywords: &["logout"],
        confirm_by_default: true,
        cannot_message: "System can't logout",
        failed_message: "Failed to log out",
    },
    PowerCommand {
        id: "suspend",
        name: "Suspend System",
        description: "Suspend the system to RAM. Unlike hibernation, this does not turn the computer off and will break on power loss.",
        keywords: &["suspend"],
        confirm_by_default: true,
        cannot_message: "System cannot suspend",
        failed_message: "Failed to suspend",
    },
    PowerCommand {
        id: "hibernate",
        name: "Hibernate System",
        description: "Suspend the system to disk. This turns off the system completely and saves its state on disk, to be restored on next boot.",
        keywords: &["disk", "suspend"],
        confirm_by_default: true,
        cannot_message: "System can't hibernate",
        failed_message: "Failed to hibernate",
    },
    PowerCommand {
        id: "soft-reboot",
        name: "Soft Reboot System",
        description: "Soft reboot the system, which usually means only userspace is rebooted.",
        keywords: &["restart"],
        confirm_by_default: true,
        cannot_message: "System can't soft reboot",
        failed_message: "Failed to soft reboot",
    },
];

/// The extension's own id, name and description.
pub const EXTENSION_ID: &str = "power";
/// The extension's display name.
pub const EXTENSION_NAME: &str = "Power Management";
/// The extension's description.
pub const EXTENSION_DESCRIPTION: &str = "Power off, suspend, sleep, hibernate your computer.";

/// The confirmation dialog's title.
pub const CONFIRM_TITLE: &str = "Are you sure";
/// The confirmation dialog's body.
pub const CONFIRM_BODY: &str = "High-impact operation, please confirm";

/// The preference that decides whether to ask.
pub const CONFIRM_PREFERENCE: &str = "confirm";
/// The preference holding a shell command to run instead.
pub const CUSTOM_PROGRAM_PREFERENCE: &str = "customProgram";
/// The `customProgram` preference's title.
pub const CUSTOM_PROGRAM_TITLE: &str = "Custom program";
/// Its description.
pub const CUSTOM_PROGRAM_DESCRIPTION: &str =
    "Custom shell command to run instead of the default implementation";

/// Finds a command by id.
#[must_use]
pub fn command(id: &str) -> Option<&'static PowerCommand> {
    COMMANDS.iter().find(|command| command.id == id)
}

/// A declared preference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Preference {
    /// `Preference::makeCheckbox`.
    Checkbox {
        /// Its name.
        name: &'static str,
        /// Its label.
        title: &'static str,
        /// Its default.
        default: bool,
    },
    /// `Preference::makeText`.
    Text {
        /// Its name.
        name: &'static str,
        /// Its label.
        title: &'static str,
        /// Its help text.
        description: &'static str,
        /// Whether the user must fill it in.
        required: bool,
    },
}

/// The preferences a power command declares.
///
/// `supportsCustomProgram()` is false on macOS and Windows and true elsewhere,
/// so the text preference only exists where a shell command makes sense.
#[must_use]
pub fn preferences(command: &PowerCommand, supports_custom_program: bool) -> Vec<Preference> {
    let mut preferences = vec![Preference::Checkbox {
        name: CONFIRM_PREFERENCE,
        title: "Ask for confirmation",
        default: command.confirm_by_default,
    }];

    if supports_custom_program {
        preferences.push(Preference::Text {
            name: CUSTOM_PROGRAM_PREFERENCE,
            title: CUSTOM_PROGRAM_TITLE,
            description: CUSTOM_PROGRAM_DESCRIPTION,
            required: false,
        });
    }

    preferences
}

/// A step in running a power command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// Ask the user first. Everything after this waits on a yes.
    Confirm {
        /// The dialog title.
        title: &'static str,
        /// The dialog body.
        body: &'static str,
    },
    /// Close the launcher window, emptying the root search.
    CloseWindow,
    /// Run the user's own shell command instead of the built-in behaviour.
    RunCustomProgram {
        /// The command line, as typed into the preference.
        program: String,
    },
    /// Do the thing the command is named after.
    Perform {
        /// Which command's action: `power-off`, `lock`, and so on.
        id: &'static str,
    },
}

/// What running `command` does, given the user's preferences.
///
/// The window is closed before the work in both paths, and in the confirming
/// path it is closed **twice** — once by the alert's callback and once at the
/// end of the shared handler. That is what the C++ does; closing an already
/// closed window is a no-op, and a port that tidied it away would be guessing
/// that no one relies on the second call.
#[must_use]
pub fn plan(command: &PowerCommand, confirm: bool, custom_program: Option<&str>) -> Vec<Step> {
    let mut steps = Vec::new();

    if confirm {
        steps.push(Step::Confirm {
            title: CONFIRM_TITLE,
            body: CONFIRM_BODY,
        });
    }

    steps.push(Step::CloseWindow);

    // `if (auto prog = prefs.value("customProgram").toString(); !prog.isEmpty())`
    // -- an empty string is no program at all, not a program called "".
    match custom_program.filter(|program| !program.is_empty()) {
        Some(program) => steps.push(Step::RunCustomProgram {
            program: program.to_owned(),
        }),
        None => steps.push(Step::Perform { id: command.id }),
    }

    steps.push(Step::CloseWindow);
    steps
}

/// What the toast says when a custom program fails.
#[must_use]
pub fn custom_program_failure(program: &str) -> String {
    format!("Failed to execute custom program {program}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eight_linux_commands_and_lock_is_only_unconfirmed() {
        assert_eq!(COMMANDS.len(), 8);
        assert!(
            COMMANDS
                .iter()
                .any(|c| c.id == "lock" && !c.confirm_by_default)
        );
        assert!(COMMANDS.iter().filter(|c| !c.confirm_by_default).count() == 1);
        assert_eq!(command("lock").unwrap().confirm_by_default, false);
        assert_eq!(command("reboot").unwrap().confirm_by_default, true);
    }

    #[test]
    fn cannot_vs_failed_messages_are_distinct_and_worded_as_ported() {
        // Check the two "cannot" wordings are reproduced, not normalized
        assert!(
            command("power-off")
                .unwrap()
                .cannot_message
                .contains("cannot")
        );
        assert!(command("reboot").unwrap().cannot_message.contains("can't"));
        assert!(custom_program_failure("myprog").contains("myprog"));
    }

    #[test]
    fn preferences_checkbox_and_custom_program() {
        let lock = command("lock").unwrap();
        let prefs_no_custom = preferences(lock, false);
        assert_eq!(prefs_no_custom.len(), 1);
        assert!(matches!(prefs_no_custom[0], Preference::Checkbox { .. }));
        let prefs_with = preferences(lock, true);
        assert_eq!(prefs_with.len(), 2);
        assert!(matches!(prefs_with[1], Preference::Text { .. }));
    }

    #[test]
    fn plan_confirm_and_custom_program_and_double_close() {
        let cmd = command("reboot").unwrap();
        let steps_confirm = plan(cmd, true, None);
        assert_eq!(
            steps_confirm[0],
            Step::Confirm {
                title: CONFIRM_TITLE,
                body: CONFIRM_BODY
            }
        );
        assert_eq!(steps_confirm[1], Step::CloseWindow);
        assert_eq!(steps_confirm[2], Step::Perform { id: "reboot" });
        assert_eq!(steps_confirm[3], Step::CloseWindow);
        // without confirm, no first Confirm
        let steps_no_confirm = plan(cmd, false, None);
        assert!(!matches!(steps_no_confirm[0], Step::Confirm { .. }));
        // custom program replaces Perform, empty string is ignored
        let steps_custom = plan(cmd, false, Some("myprog --arg"));
        assert!(matches!(steps_custom[1], Step::RunCustomProgram { .. }));
        let steps_empty = plan(cmd, false, Some(""));
        assert!(matches!(steps_empty[1], Step::Perform { .. }));
    }

    #[test]
    fn command_lookup_and_extension_meta() {
        assert!(command("power-off").is_some());
        assert!(command("missing").is_none());
        assert_eq!(EXTENSION_ID, "power");
        assert_eq!(CONFIRM_TITLE, "Are you sure");
    }
}
