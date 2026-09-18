//! The "run a program" builtin's model: a typed command line, the programs on
//! `PATH` that match it, and how each can be run.
//!
//! A port of `system-run-model.cpp`
//! (`src/server/src/builtins/system/`). Two list sections — the command line as
//! typed, and matching executables — with the same three-way choice of how to
//! run each: in a terminal that stays open, in one that closes, or directly.
//!
//! Which of the three is *first* is a preference, and the C++ implements it
//! with a swap rather than a rotation. That is not a detail a rewrite can
//! guess, so it has a test.

/// What the user's `defaultAction` preference does to the order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DefaultAction {
    /// Run in a terminal that closes when the command exits.
    RunInTerminal,
    /// Run in a terminal that stays open; the C++ `default:` arm, and what the
    /// unswapped list already starts with.
    #[default]
    RunInTerminalHold,
    /// Run the program directly, with no terminal.
    Run,
}

impl DefaultAction {
    /// `parseSystemRunDefaultAction`: two names, and anything else is [`Self::Run`].
    ///
    /// Note that the fallback is *not* [`Self::RunInTerminalHold`], even though
    /// that is the enum's first value and the order's natural default — an
    /// unreadable preference means "just run it".
    #[must_use]
    pub fn parse(value: &str) -> Self {
        match value {
            "run-in-terminal" => Self::RunInTerminal,
            "run-in-terminal-hold" => Self::RunInTerminalHold,
            _ => Self::Run,
        }
    }
}

/// One way of running the selected thing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunAction {
    /// `OpenInTerminalAction` with the default `hold`.
    OpenInTerminalHold,
    /// `OpenInTerminalAction` with `{.hold = false}`.
    OpenInTerminal,
    /// `OpenRawProgramAction`.
    ExecuteProgram,
}

/// `OpenRawProgramAction::title()`.
pub const EXECUTE_TITLE: &str = "Execute program";

/// The title of the action that copies an executable's path.
pub const COPY_EXEC_PATH_TITLE: &str = "Copy exec path";

impl RunAction {
    /// The action's title, given the terminal's display name.
    ///
    /// `tr("Open in %1 (hold)")` and `tr("Open in %1")`; the raw one has a
    /// fixed title of its own.
    #[must_use]
    pub fn title(self, terminal: &str) -> String {
        match self {
            Self::OpenInTerminalHold => format!("Open in {terminal} (hold)"),
            Self::OpenInTerminal => format!("Open in {terminal}"),
            Self::ExecuteProgram => EXECUTE_TITLE.to_owned(),
        }
    }
}

/// The three run actions, ordered by `default_action`.
///
/// The C++ builds `{hold, noHold, runRaw}` and then swaps the *first* element
/// with the preferred one, so choosing "run" gives `{runRaw, noHold, hold}` —
/// the two it did not choose end up in the opposite order from where they
/// started. A rotation would give `{runRaw, hold, noHold}`, which is a
/// different menu.
#[must_use]
pub fn run_actions(default_action: DefaultAction) -> [RunAction; 3] {
    let mut actions = [
        RunAction::OpenInTerminalHold,
        RunAction::OpenInTerminal,
        RunAction::ExecuteProgram,
    ];

    let preferred = match default_action {
        DefaultAction::RunInTerminal => Some(RunAction::OpenInTerminal),
        DefaultAction::Run => Some(RunAction::ExecuteProgram),
        // The `default:` arm swaps nothing.
        DefaultAction::RunInTerminalHold => None,
    };

    if let Some(preferred) = preferred
        && let Some(index) = actions.iter().position(|action| *action == preferred)
    {
        actions.swap(0, index);
    }

    actions
}

/// The command-line row: what the user typed, run as a command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandLineRow {
    /// The words, joined with single spaces — `views::join_with(' ')`.
    pub title: String,
    /// The actions offered, in order. Empty when there is no terminal at all.
    pub actions: Vec<RunAction>,
}

/// Builds the command-line row.
///
/// With no terminal emulator the C++ adds **nothing**: the whole `if (terminal)`
/// block is the panel, so the row offers no way to run the command — not even
/// the raw one, which needs no terminal. Reproduced, because a Rust host that
/// offered "Execute program" there would run commands the C++ host refuses to.
#[must_use]
pub fn command_line_row(
    cmdline: &[String],
    terminal: bool,
    default_action: DefaultAction,
) -> CommandLineRow {
    CommandLineRow {
        title: cmdline.join(" "),
        actions: if terminal {
            run_actions(default_action).to_vec()
        } else {
            Vec::new()
        },
    }
}

/// A row for one executable found on `PATH`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProgramRow {
    /// The file name alone.
    pub title: String,
    /// The path, with `$HOME` folded to `~`.
    pub subtitle: String,
    /// The run actions, then "Copy exec path" — which is outside the
    /// `if (terminal)` block, so it is there either way.
    pub actions: Vec<ProgramAction>,
}

/// An action on a program row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgramAction {
    /// One of the three ways to run it.
    Run(RunAction),
    /// Copy its path to the clipboard.
    CopyExecPath,
}

/// `compressPath`: a path under `$HOME` becomes `~/...`, anything else is left
/// alone.
///
/// The test is `str.starts_with(homeStr)` on the strings, with no check that
/// the next character is a separator — so with `$HOME` at `/home/ana`, the path
/// `/home/anastasia/bin/x` displays as `~stasia/bin/x`. Reproduced and pinned:
/// it is cosmetic, it only bites users whose home directory name is a prefix of
/// a sibling's, and a subtitle that differs between the two engines would be a
/// worse surprise than a wrong one that matches.
#[must_use]
pub fn compress_path(path: &str, home: &str) -> String {
    if path.starts_with(home) {
        format!("~{}", &path[home.len()..])
    } else {
        path.to_owned()
    }
}

/// Builds a program row.
#[must_use]
pub fn program_row(
    path: &str,
    home: &str,
    terminal: bool,
    default_action: DefaultAction,
) -> ProgramRow {
    let mut actions: Vec<ProgramAction> = if terminal {
        run_actions(default_action)
            .into_iter()
            .map(ProgramAction::Run)
            .collect()
    } else {
        Vec::new()
    };
    actions.push(ProgramAction::CopyExecPath);

    ProgramRow {
        title: path.rsplit('/').next().unwrap_or(path).to_owned(),
        subtitle: compress_path(path, home),
        actions,
    }
}
