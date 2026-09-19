//! The "run a program" builtin, read against
//! `src/server/src/builtins/system/system-run-model.cpp`.

use compass_core::system_run::{
    COPY_EXEC_PATH_TITLE, DefaultAction, EXECUTE_TITLE, ProgramAction, RunAction, command_line_row,
    compress_path, program_row, run_actions,
};

#[test]
fn the_preference_is_parsed_by_name_and_anything_else_means_run() {
    // `parseSystemRunDefaultAction`: two names, then `return Run`.
    assert_eq!(
        DefaultAction::parse("run-in-terminal"),
        DefaultAction::RunInTerminal
    );
    assert_eq!(
        DefaultAction::parse("run-in-terminal-hold"),
        DefaultAction::RunInTerminalHold
    );

    for unknown in ["", "run", "Run", "open-in-terminal", "nonsense"] {
        assert_eq!(
            DefaultAction::parse(unknown),
            DefaultAction::Run,
            "{unknown:?} should fall through to Run"
        );
    }
}

#[test]
fn the_default_order_is_hold_then_no_hold_then_raw() {
    // `std::array actions = {hold, noHold, runRaw};` with the `default:` arm
    // swapping nothing.
    assert_eq!(
        run_actions(DefaultAction::RunInTerminalHold),
        [
            RunAction::OpenInTerminalHold,
            RunAction::OpenInTerminal,
            RunAction::ExecuteProgram,
        ]
    );
}

#[test]
fn preferring_the_raw_run_swaps_rather_than_rotates() {
    // `std::iter_swap(actions.begin(), std::ranges::find(actions, runRaw))`
    // gives {runRaw, noHold, hold} -- the two it did not choose end up in the
    // opposite order. A rotation would give {runRaw, hold, noHold}, which is a
    // different menu under the user's fingers.
    assert_eq!(
        run_actions(DefaultAction::Run),
        [
            RunAction::ExecuteProgram,
            RunAction::OpenInTerminal,
            RunAction::OpenInTerminalHold,
        ]
    );
}

#[test]
fn preferring_the_closing_terminal_swaps_the_first_two() {
    // The same swap, where the preferred item is already second: {noHold, hold,
    // runRaw}.
    assert_eq!(
        run_actions(DefaultAction::RunInTerminal),
        [
            RunAction::OpenInTerminal,
            RunAction::OpenInTerminalHold,
            RunAction::ExecuteProgram,
        ]
    );
}

#[test]
fn the_terminal_actions_are_named_after_the_terminal() {
    // `tr("Open in %1 (hold)").arg(terminal->displayName())`.
    assert_eq!(
        RunAction::OpenInTerminalHold.title("Console"),
        "Open in Console (hold)"
    );
    assert_eq!(
        RunAction::OpenInTerminal.title("Console"),
        "Open in Console"
    );
    assert_eq!(
        RunAction::ExecuteProgram.title("Console"),
        EXECUTE_TITLE,
        "the raw one has a title of its own"
    );
    assert_eq!(EXECUTE_TITLE, "Execute program");
}

#[test]
fn the_command_line_row_is_the_words_joined_with_spaces() {
    // `m_cmdline | std::views::join_with(' ')`.
    let row = command_line_row(
        &["ls".to_owned(), "-la".to_owned(), "/tmp".to_owned()],
        true,
        DefaultAction::default(),
    );
    assert_eq!(row.title, "ls -la /tmp");
}

#[test]
fn without_a_terminal_the_command_line_offers_nothing_at_all() {
    // The whole panel is inside `if (terminal)`, so a machine with no terminal
    // emulator gets a row it cannot act on -- not even "Execute program",
    // which needs no terminal. Reproduced: offering it here would run commands
    // the C++ host refuses to.
    let row = command_line_row(&["ls".to_owned()], false, DefaultAction::Run);

    assert_eq!(row.title, "ls");
    assert!(row.actions.is_empty());
}

#[test]
fn a_program_row_shows_the_file_name_over_the_compressed_path() {
    let row = program_row(
        "/home/ana/.local/bin/mytool",
        "/home/ana",
        true,
        DefaultAction::default(),
    );

    assert_eq!(row.title, "mytool");
    assert_eq!(row.subtitle, "~/.local/bin/mytool");
}

#[test]
fn compressing_a_path_is_a_string_prefix_test_and_that_shows() {
    // `if (str.starts_with(homeStr)) return "~" + str.substr(homeStr.size());`
    // -- no separator check, so a sibling home directory whose name starts with
    // yours is mangled. Cosmetic, reproduced, and pinned so it is a decision.
    assert_eq!(compress_path("/home/ana/x", "/home/ana"), "~/x");
    assert_eq!(compress_path("/usr/bin/x", "/home/ana"), "/usr/bin/x");
    assert_eq!(
        compress_path("/home/anastasia/bin/x", "/home/ana"),
        "~stasia/bin/x",
        "the C++ does this too"
    );
}

#[test]
fn copy_exec_path_is_offered_even_when_there_is_no_terminal() {
    // It is added *outside* the `if (terminal)` block, unlike the command-line
    // row's actions.
    let with = program_row("/usr/bin/ls", "/home/ana", true, DefaultAction::default());
    let without = program_row("/usr/bin/ls", "/home/ana", false, DefaultAction::default());

    assert_eq!(with.actions.len(), 4);
    assert_eq!(with.actions[3], ProgramAction::CopyExecPath);
    assert_eq!(without.actions, [ProgramAction::CopyExecPath]);
    assert_eq!(COPY_EXEC_PATH_TITLE, "Copy exec path");
}

#[test]
fn a_program_rows_run_actions_follow_the_same_preference() {
    let row = program_row("/usr/bin/ls", "/home/ana", true, DefaultAction::Run);

    assert_eq!(
        row.actions[..3],
        [
            ProgramAction::Run(RunAction::ExecuteProgram),
            ProgramAction::Run(RunAction::OpenInTerminal),
            ProgramAction::Run(RunAction::OpenInTerminalHold),
        ]
    );
}
