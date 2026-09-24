//! How to run a command inside a terminal emulator.
//!
//! Ports `XdgAppDatabase::inferTermExec` and the `X-TerminalArg*` keys from
//! `src/lib/xdgpp/xdgpp/desktop-entry/entry.hpp`.
//!
//! Every terminal emulator spells "run this command" differently, and several
//! spell the other options differently again. There is no specification for
//! any of it, so this is a table of what each one actually accepts — the kind
//! of thing where one wrong flag breaks "open in terminal" for one emulator
//! and nobody notices, because nobody has all of them installed.

/// The flags one terminal emulator accepts.
///
/// Every field is optional and an absent one means *this terminal has no such
/// flag*, not "use the default". Passing `--title` to a terminal that does not
/// take it does not get an untitled window, it gets an error and no window at
/// all, so the absence has to survive into the caller.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TerminalArgs {
    /// The flag before the command to run. Every terminal has one.
    pub exec: Option<String>,
    /// The flag that sets the window's application id.
    pub app_id: Option<String>,
    /// The flag that sets the window title.
    pub title: Option<String>,
    /// The flag that sets the working directory.
    pub dir: Option<String>,
    /// The flag that keeps the window open after the command exits.
    pub hold: Option<String>,
}

impl TerminalArgs {
    /// A terminal that only knows how to run a command.
    fn exec_only(exec: &str) -> Self {
        Self {
            exec: Some(exec.to_owned()),
            ..Self::default()
        }
    }
}

/// The desktop-entry keys that override the table.
pub const EXEC_KEY: &str = "X-TerminalArgExec";
/// The key naming the application-id flag.
pub const APP_ID_KEY: &str = "X-TerminalArgAppId";
/// The key naming the title flag.
pub const TITLE_KEY: &str = "X-TerminalArgTitle";
/// The key naming the working-directory flag.
pub const DIR_KEY: &str = "X-TerminalArgDir";
/// The key naming the hold flag.
pub const HOLD_KEY: &str = "X-TerminalArgHold";

/// Read the `X-TerminalArg*` keys a terminal's own desktop file declares.
///
/// `X-TerminalArgExec` is the gate: without it there is no override at all,
/// whatever else is present. A file naming only a title flag is not describing
/// a terminal this can drive — it cannot say how to run the command, which is
/// the one thing every caller needs.
#[must_use]
pub fn declared_args(lookup: impl Fn(&str) -> Option<String>) -> Option<TerminalArgs> {
    let exec = lookup(EXEC_KEY)?;
    Some(TerminalArgs {
        exec: Some(exec),
        app_id: lookup(APP_ID_KEY),
        title: lookup(TITLE_KEY),
        dir: lookup(DIR_KEY),
        hold: lookup(HOLD_KEY),
    })
}

/// What each known terminal accepts, keyed on the *program* its `Exec` starts
/// with.
///
/// Keyed on the program and not the desktop id, because the same emulator
/// ships under different ids on different distributions while the binary keeps
/// its name.
#[must_use]
pub fn known_args(program: &str) -> Option<TerminalArgs> {
    Some(match program {
        // The old GNOME Terminal takes `--` and nothing else this cares about.
        "gnome-terminal" => TerminalArgs::exec_only("--"),
        // The new one (Console) takes more — and its working-directory flag
        // has **no leading dashes**, which is not a typo here: `kgx` accepts
        // `working-directory` as written.
        "kgx" => TerminalArgs {
            exec: Some("--".to_owned()),
            title: Some("--title".to_owned()),
            dir: Some("working-directory".to_owned()),
            ..TerminalArgs::default()
        },
        "alacritty" => TerminalArgs {
            exec: Some("-e".to_owned()),
            app_id: Some("--class".to_owned()),
            title: Some("--title".to_owned()),
            dir: Some("--working-directory".to_owned()),
            hold: Some("--hold".to_owned()),
        },
        "cosmic-term" => TerminalArgs::exec_only("-e"),
        "konsole" => TerminalArgs {
            exec: Some("-e".to_owned()),
            dir: Some("--workdir".to_owned()),
            hold: Some("--hold".to_owned()),
            ..TerminalArgs::default()
        },
        // `footclient` takes the same flags as `foot`; it is the client half
        // of the same program.
        "foot" | "footclient" => TerminalArgs {
            exec: Some("-e".to_owned()),
            app_id: Some("--app-id".to_owned()),
            title: Some("--title".to_owned()),
            dir: Some("--working-directory".to_owned()),
            hold: Some("--hold".to_owned()),
        },
        // Both descend from the same GNOME Terminal fork and both kept `-x`
        // where GNOME moved to `--`.
        "mate-terminal" | "xfce4-terminal" => TerminalArgs::exec_only("-x"),
        _ => return None,
    })
}

/// The flag an unknown terminal is assumed to take.
///
/// `-e` is what most of them accept, and guessing is better than refusing:
/// a terminal nobody has added to the table still opens, it just gets no
/// title, directory or hold. Guessing *more* than `-e` would not be — a wrong
/// `--title` is an error and no window, where a missing one is a window with
/// the wrong name.
#[must_use]
pub fn fallback_args() -> TerminalArgs {
    TerminalArgs::exec_only("-e")
}

/// The flags to drive a terminal with.
///
/// The terminal's own desktop file wins, then the table, then the guess. The
/// file comes first because it is the only source that can be right about a
/// terminal released after this table was written.
#[must_use]
pub fn terminal_args(declared: Option<TerminalArgs>, program: &str) -> TerminalArgs {
    declared
        .or_else(|| known_args(program))
        .unwrap_or_else(fallback_args)
}

/// Build the argument vector that runs `command` in a terminal.
///
/// Each option is included only when the terminal has a flag for it *and* the
/// caller asked for it. The command goes last, after its `exec` flag, because
/// every one of these terminals treats everything after that flag as the
/// command — so anything appended afterwards would be passed to the command
/// rather than to the terminal.
#[must_use]
pub fn terminal_command(
    terminal: &str,
    args: &TerminalArgs,
    command: &[String],
    title: Option<&str>,
    dir: Option<&str>,
    app_id: Option<&str>,
    hold: bool,
) -> Vec<String> {
    let mut out = vec![terminal.to_owned()];
    // Per the xdg-terminal-exec spec, a flag ending in `=` takes its value in
    // the same argument (`--working-directory=/home`), as the C++ does.
    let mut flag = |flag: &str, value: &str| {
        if flag.ends_with('=') {
            out.push(format!("{flag}{value}"));
        } else {
            out.push(flag.to_owned());
            out.push(value.to_owned());
        }
    };

    if let (Some(name), Some(value)) = (&args.app_id, app_id) {
        flag(name, value);
    }
    if let (Some(name), Some(value)) = (&args.title, title) {
        flag(name, value);
    }
    if let (Some(name), Some(value)) = (&args.dir, dir) {
        flag(name, value);
    }
    if hold && let Some(flag) = &args.hold {
        out.push(flag.clone());
    }
    if let Some(flag) = &args.exec {
        out.push(flag.clone());
    }
    out.extend(command.iter().cloned());
    out
}

/// How an `xdg-terminals.list` line ranks its terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListState {
    /// Chosen: the first one that is installed is the terminal.
    Selected,
    /// `+id`: kept out of the fallback's exclusions, not chosen.
    Protected,
    /// `-id`: never the fallback.
    Excluded,
}

/// One `xdg-terminals.list` entry: a desktop-file id, an optional action,
/// and its state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListEntry {
    /// The desktop-file id, e.g. `org.gnome.Ptyxis.desktop`.
    pub id: String,
    /// The `:action`, if the line names one.
    pub action: Option<String>,
    /// Chosen, protected or excluded.
    pub state: ListState,
}

/// The entries of one `xdg-terminals.list`, in order. Ports
/// `parseXdgTerminalsList`: blank lines, comments and anything that does not
/// name a `.desktop` are skipped.
#[must_use]
pub fn parse_terminals_list(text: &str) -> Vec<ListEntry> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#') && line.contains(".desktop"))
        .map(|line| {
            let (state, raw) = if let Some(raw) = line.strip_prefix('+') {
                (ListState::Protected, raw)
            } else if let Some(raw) = line.strip_prefix('-') {
                (ListState::Excluded, raw)
            } else {
                (ListState::Selected, line)
            };
            let (id, action) = match raw.split_once(':') {
                Some((id, action)) => (id, Some(action.to_owned())),
                None => (raw, None),
            };
            ListEntry {
                id: id.to_owned(),
                action,
                state,
            }
        })
        .collect()
}

/// Where `xdg-terminals.list` files are read, first wins: each config
/// directory's desktop-prefixed then plain list, then each data directory's
/// `xdg-terminal-exec/` fallbacks. Ports `xdgTerminalsListPaths`.
#[must_use]
pub fn terminals_list_paths(
    config_home: Option<&std::path::Path>,
    config_dirs: &[std::path::PathBuf],
    data_dirs: &[std::path::PathBuf],
    desktops: &[String],
) -> Vec<std::path::PathBuf> {
    let desktops: Vec<String> = desktops.iter().map(|d| d.to_lowercase()).collect();
    let mut paths = Vec::new();
    for dir in config_home
        .into_iter()
        .chain(config_dirs.iter().map(std::path::PathBuf::as_path))
    {
        for desktop in &desktops {
            paths.push(dir.join(format!("{desktop}-xdg-terminals.list")));
        }
        paths.push(dir.join("xdg-terminals.list"));
    }
    for dir in data_dirs {
        for desktop in &desktops {
            paths.push(
                dir.join("xdg-terminal-exec")
                    .join(format!("{desktop}-xdg-terminals.list")),
            );
        }
        paths.push(dir.join("xdg-terminal-exec/xdg-terminals.list"));
    }
    paths
}
