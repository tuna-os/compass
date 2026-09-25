//! How to run a command inside a terminal emulator.
//!
//! Ported from `XdgAppDatabase::inferTermExec` and the `X-TerminalArg*` keys.

use compass_xdg::terminal::{
    APP_ID_KEY, DIR_KEY, EXEC_KEY, HOLD_KEY, TITLE_KEY, TerminalArgs, declared_args, fallback_args,
    known_args, terminal_args, terminal_command,
};

fn cmd(words: &[&str]) -> Vec<String> {
    words.iter().map(|w| (*w).to_owned()).collect()
}

// --- the table ----------------------------------------------------------

#[test]
fn the_old_gnome_terminal_takes_only_a_double_dash() {
    let args = known_args("gnome-terminal").expect("known");
    assert_eq!(args.exec.as_deref(), Some("--"));
    assert_eq!(args.title, None);
    assert_eq!(args.dir, None);
    assert_eq!(args.hold, None);
}

#[test]
fn the_new_gnome_console_takes_more_than_the_old_one() {
    // They are different programs under different names, and conflating them
    // would lose the title and directory on the one that has them.
    let kgx = known_args("kgx").expect("known");
    let old = known_args("gnome-terminal").expect("known");
    assert_eq!(kgx.exec.as_deref(), Some("--"));
    assert_eq!(kgx.title.as_deref(), Some("--title"));
    assert!(old.title.is_none());
}

#[test]
fn the_consoles_directory_flag_has_no_leading_dashes() {
    // Not a typo: `kgx` accepts `working-directory` as written, and "fixing"
    // it to `--working-directory` is the kind of tidying that breaks one
    // terminal silently.
    assert_eq!(
        known_args("kgx").expect("known").dir.as_deref(),
        Some("working-directory")
    );
}

#[test]
fn alacritty_takes_the_full_set() {
    let args = known_args("alacritty").expect("known");
    assert_eq!(args.exec.as_deref(), Some("-e"));
    assert_eq!(args.app_id.as_deref(), Some("--class"));
    assert_eq!(args.title.as_deref(), Some("--title"));
    assert_eq!(args.dir.as_deref(), Some("--working-directory"));
    assert_eq!(args.hold.as_deref(), Some("--hold"));
}

#[test]
fn konsole_has_a_hold_and_a_directory_but_no_title_or_app_id() {
    // The gaps are the point: passing `--title` to konsole is an error and no
    // window, not an untitled one.
    let args = known_args("konsole").expect("known");
    assert_eq!(args.dir.as_deref(), Some("--workdir"));
    assert_eq!(args.hold.as_deref(), Some("--hold"));
    assert_eq!(args.title, None);
    assert_eq!(args.app_id, None);
}

#[test]
fn foot_and_its_client_take_the_same_flags() {
    // `footclient` is the client half of the same program.
    assert_eq!(known_args("foot"), known_args("footclient"));
    assert_eq!(
        known_args("foot").expect("known").app_id.as_deref(),
        Some("--app-id")
    );
}

#[test]
fn the_two_gnome_terminal_forks_kept_the_flag_gnome_moved_away_from() {
    assert_eq!(
        known_args("mate-terminal").expect("known").exec.as_deref(),
        Some("-x")
    );
    assert_eq!(
        known_args("xfce4-terminal").expect("known").exec.as_deref(),
        Some("-x")
    );
}

#[test]
fn cosmic_term_takes_the_common_flag() {
    assert_eq!(
        known_args("cosmic-term").expect("known").exec.as_deref(),
        Some("-e")
    );
}

#[test]
fn a_terminal_nobody_listed_is_not_in_the_table() {
    assert_eq!(known_args("wezterm"), None);
    assert_eq!(known_args(""), None);
}

#[test]
fn the_table_is_keyed_on_the_program_and_not_a_desktop_id() {
    // The same emulator ships under different ids on different distributions
    // while the binary keeps its name.
    assert!(known_args("org.gnome.Console").is_none());
    assert!(known_args("kgx").is_some());
}

// --- the fallback -------------------------------------------------------

#[test]
fn an_unknown_terminal_is_guessed_at_rather_than_refused() {
    // It still opens; it just gets no title, directory or hold.
    let args = fallback_args();
    assert_eq!(args.exec.as_deref(), Some("-e"));
}

#[test]
fn the_guess_claims_nothing_beyond_running_the_command() {
    // A wrong `--title` is an error and no window, where a missing one is a
    // window with the wrong name. So the guess is deliberately minimal.
    let args = fallback_args();
    assert_eq!(args.title, None);
    assert_eq!(args.app_id, None);
    assert_eq!(args.dir, None);
    assert_eq!(args.hold, None);
}

// --- what a terminal's own desktop file declares ------------------------

fn declared(pairs: &[(&'static str, &'static str)]) -> Option<TerminalArgs> {
    let owned: Vec<(String, String)> = pairs
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect();
    declared_args(|key: &str| {
        owned
            .iter()
            .find(|(known, _)| known == key)
            .map(|(_, value)| value.clone())
    })
}

#[test]
fn the_exec_key_is_the_gate() {
    // A file naming only a title flag is not describing a terminal this can
    // drive: it cannot say how to run the command, which is the one thing
    // every caller needs.
    assert_eq!(declared(&[(TITLE_KEY, "--title")]), None);
    assert!(declared(&[(EXEC_KEY, "-e")]).is_some());
}

#[test]
fn every_declared_key_is_read() {
    let args = declared(&[
        (EXEC_KEY, "-e"),
        (APP_ID_KEY, "--class="),
        (TITLE_KEY, "--title="),
        (DIR_KEY, "--working-directory="),
        (HOLD_KEY, "--wait-after-command"),
    ])
    .expect("declared");
    assert_eq!(args.exec.as_deref(), Some("-e"));
    assert_eq!(args.app_id.as_deref(), Some("--class="));
    assert_eq!(args.title.as_deref(), Some("--title="));
    assert_eq!(args.dir.as_deref(), Some("--working-directory="));
    assert_eq!(args.hold.as_deref(), Some("--wait-after-command"));
}

#[test]
fn a_file_declaring_only_the_exec_key_declares_only_that() {
    let args = declared(&[(EXEC_KEY, "-e")]).expect("declared");
    assert_eq!(args.exec.as_deref(), Some("-e"));
    assert_eq!(args.title, None);
}

// --- which source wins --------------------------------------------------

#[test]
fn a_terminals_own_file_beats_the_table() {
    // It is the only source that can be right about a terminal released after
    // the table was written — including a new version of a listed one.
    let chosen = terminal_args(declared(&[(EXEC_KEY, "--new-flag")]), "konsole");
    assert_eq!(chosen.exec.as_deref(), Some("--new-flag"));
}

#[test]
fn the_table_beats_the_guess() {
    assert_eq!(
        terminal_args(None, "alacritty").app_id.as_deref(),
        Some("--class")
    );
}

#[test]
fn the_guess_is_the_last_resort() {
    let chosen = terminal_args(None, "wezterm");
    assert_eq!(chosen, fallback_args());
}

// --- building the command line ------------------------------------------

#[test]
fn the_command_goes_last_after_its_flag() {
    // Every one of these terminals treats everything after the exec flag as
    // the command, so anything appended afterwards would reach the command
    // rather than the terminal.
    let line = terminal_command(
        "alacritty",
        &known_args("alacritty").expect("known"),
        &cmd(&["htop", "-d", "5"]),
        Some("Monitor"),
        None,
        None,
        false,
    );
    assert_eq!(
        line,
        cmd(&["alacritty", "--title", "Monitor", "-e", "htop", "-d", "5"])
    );
}

#[test]
fn an_option_the_terminal_cannot_take_is_left_out_entirely() {
    // Passing it anyway is an error and no window.
    let line = terminal_command(
        "konsole",
        &known_args("konsole").expect("known"),
        &cmd(&["htop"]),
        Some("Monitor"),
        None,
        None,
        false,
    );
    assert!(!line.iter().any(|w| w == "Monitor"), "{line:?}");
    assert_eq!(line, cmd(&["konsole", "-e", "htop"]));
}

#[test]
fn an_option_the_caller_did_not_ask_for_is_left_out_too() {
    let line = terminal_command(
        "alacritty",
        &known_args("alacritty").expect("known"),
        &cmd(&["htop"]),
        None,
        None,
        None,
        false,
    );
    assert_eq!(line, cmd(&["alacritty", "-e", "htop"]));
}

#[test]
fn every_option_the_terminal_takes_can_be_used_at_once() {
    let line = terminal_command(
        "foot",
        &known_args("foot").expect("known"),
        &cmd(&["htop"]),
        Some("Monitor"),
        Some("/tmp"),
        Some("compass"),
        true,
    );
    assert_eq!(
        line,
        cmd(&[
            "foot",
            "--app-id",
            "compass",
            "--title",
            "Monitor",
            "--working-directory",
            "/tmp",
            "--hold",
            "-e",
            "htop"
        ])
    );
}

#[test]
fn hold_is_a_flag_on_its_own_with_no_value() {
    let line = terminal_command(
        "konsole",
        &known_args("konsole").expect("known"),
        &cmd(&["htop"]),
        None,
        None,
        None,
        true,
    );
    assert_eq!(line, cmd(&["konsole", "--hold", "-e", "htop"]));
}

#[test]
fn asking_to_hold_a_terminal_that_cannot_is_not_an_error() {
    // It closes when the command exits, which is the behaviour without the
    // flag — and better than refusing to open.
    let line = terminal_command(
        "gnome-terminal",
        &known_args("gnome-terminal").expect("known"),
        &cmd(&["htop"]),
        None,
        None,
        None,
        true,
    );
    assert_eq!(line, cmd(&["gnome-terminal", "--", "htop"]));
}

#[test]
fn an_empty_command_still_opens_the_terminal() {
    let line = terminal_command(
        "alacritty",
        &known_args("alacritty").expect("known"),
        &[],
        None,
        None,
        None,
        false,
    );
    assert_eq!(line, cmd(&["alacritty", "-e"]));
}

#[test]
fn a_command_with_arguments_keeps_them_in_order() {
    let line = terminal_command(
        "xfce4-terminal",
        &known_args("xfce4-terminal").expect("known"),
        &cmd(&["sh", "-c", "echo one two"]),
        None,
        None,
        None,
        false,
    );
    assert_eq!(
        line,
        cmd(&["xfce4-terminal", "-x", "sh", "-c", "echo one two"])
    );
}

// --- xdg-terminals.list -------------------------------------------------

#[test]
fn a_flag_ending_in_equals_takes_its_value_in_the_same_argument() {
    let args = TerminalArgs {
        exec: Some("--".to_owned()),
        dir: Some("--working-directory=".to_owned()),
        title: Some("--title".to_owned()),
        ..TerminalArgs::default()
    };
    assert_eq!(
        terminal_command(
            "ptyxis",
            &args,
            &cmd(&["htop"]),
            Some("Top"),
            Some("/home/u"),
            None,
            false
        ),
        cmd(&[
            "ptyxis",
            "--title",
            "Top",
            "--working-directory=/home/u",
            "--",
            "htop"
        ])
    );
}

#[test]
fn a_terminals_list_is_read_as_the_c_plus_plus_reads_it() {
    use compass_xdg::terminal::{ListEntry, ListState, parse_terminals_list};
    let entries = parse_terminals_list(
        "# Configured by hand\n\n\
         org.gnome.Ptyxis.desktop\n\
         -xterm.desktop\n\
         +foot.desktop:server\n\
         not-a-terminal\n",
    );
    assert_eq!(
        entries,
        [
            ListEntry {
                id: "org.gnome.Ptyxis.desktop".into(),
                action: None,
                state: ListState::Selected
            },
            ListEntry {
                id: "xterm.desktop".into(),
                action: None,
                state: ListState::Excluded
            },
            ListEntry {
                id: "foot.desktop".into(),
                action: Some("server".into()),
                state: ListState::Protected
            },
        ]
    );
}

#[test]
fn the_lists_are_looked_for_in_config_then_data_fallbacks() {
    use compass_xdg::terminal::terminals_list_paths;
    use std::path::{Path, PathBuf};
    let paths = terminals_list_paths(
        Some(Path::new("/home/u/.config")),
        &[PathBuf::from("/etc/xdg")],
        &[PathBuf::from("/usr/share")],
        &["GNOME".to_owned()],
    );
    let paths: Vec<String> = paths.iter().map(|p| p.display().to_string()).collect();
    assert_eq!(
        paths,
        [
            "/home/u/.config/gnome-xdg-terminals.list",
            "/home/u/.config/xdg-terminals.list",
            "/etc/xdg/gnome-xdg-terminals.list",
            "/etc/xdg/xdg-terminals.list",
            "/usr/share/xdg-terminal-exec/gnome-xdg-terminals.list",
            "/usr/share/xdg-terminal-exec/xdg-terminals.list",
        ]
    );
}

// --- choosing the terminal: `src/lib/xdgpp/tests/xdg-terminal-exec.cpp` --

#[test]
fn a_chosen_terminal_is_written_to_an_empty_file() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let path = dir.path().join("xdg-terminals.list");
    compass_xdg::terminal::set_default_terminal(&path, "test", None).expect("written");
    assert_eq!(
        std::fs::read_to_string(&path).expect("read back"),
        "# Configured by the Compass launcher\ntest\n"
    );
}

#[test]
fn a_chosen_terminal_goes_above_every_existing_entry() {
    assert_eq!(
        compass_xdg::terminal::with_default_terminal("org.someone.something\n", "test", None),
        "# Configured by the Compass launcher\ntest\norg.someone.something\n"
    );
}

#[test]
fn choosing_again_replaces_the_previous_choice_and_keeps_comments() {
    assert_eq!(
        compass_xdg::terminal::with_default_terminal(
            "# Configured by the Compass launcher\n# This is some comment\n\
             org.someone.something\norg.somethingelse.unrelated\n",
            "test",
            None
        ),
        "# Configured by the Compass launcher\n# This is some comment\n\
         test\norg.somethingelse.unrelated\n"
    );
}

#[test]
fn the_pre_rename_header_is_recognised_and_rewritten() {
    assert_eq!(
        compass_xdg::terminal::with_default_terminal(
            "# Configured by the Vicinae launcher\norg.someone.something\nother\n",
            "test",
            None
        ),
        "# Configured by the Compass launcher\ntest\nother\n"
    );
}

#[test]
fn a_chosen_action_is_written_after_a_colon_and_reads_back() {
    let written =
        compass_xdg::terminal::with_default_terminal("", "kitty.desktop", Some("new-window"));
    let entries = compass_xdg::terminal::parse_terminals_list(&written);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].id, "kitty.desktop");
    assert_eq!(entries[0].action.as_deref(), Some("new-window"));
}
