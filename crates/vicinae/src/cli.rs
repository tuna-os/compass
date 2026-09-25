//! The `vicinae` command-line surface.
//!
//! Kept deliberately close to the C++ CLI in `src/cli/` so muscle memory keeps
//! working: `toggle`, `ping` and the socket semantics are the same, and the
//! C++ spellings `open`/`close` survive as aliases of `show`/`hide`.
//!
//! This is the Phase 2 slice only (PLAN §6): window control, liveness and the
//! diagnostic. `launch`, `cmd`, `dmenu`, `deeplink` and `logs` stay with the
//! C++ binary until their parity-ledger rows go green.

use std::path::PathBuf;

use clap::{Parser, Subcommand};
use compass_ipc::SocketPath;

use crate::engine::Engine;

/// Long help shared by the top level and `doctor`, so `vicinae --help` and
/// `vicinae doctor --help` both document the exit codes.
pub const EXIT_CODE_HELP: &str = "\
Exit codes:
  0  the command succeeded; for `doctor --check-only`, no check failed
     (warnings alone still exit 0 — they are supported degradations)
  1  the command failed, or `doctor --check-only` found a failing check
  2  the command line could not be parsed";

/// Compass launcher control, from the shell.
#[derive(Debug, Parser)]
#[command(
    name = "vicinae",
    version,
    about = "Control and diagnose the Compass launcher",
    after_help = EXIT_CODE_HELP,
    after_long_help = EXIT_CODE_HELP,
)]
pub struct Cli {
    /// Engine implementation to talk to.
    ///
    /// This binary is the Rust engine and cannot dispatch to the C++ one; see
    /// `PLAN.md` §5. `--engine cpp` parses, is reported by `doctor`, and makes
    /// engine-dependent commands refuse rather than quietly do the Rust thing.
    #[arg(
        long,
        global = true,
        value_enum,
        env = "COMPASS_ENGINE",
        default_value_t = Engine::Rust,
    )]
    pub engine: Engine,

    /// Path to the IPC socket, overriding `$XDG_RUNTIME_DIR/vicinae/ipc.sock`.
    ///
    /// Used verbatim: no `vicinae/` component is appended. This is how tests
    /// and a second instance stay out of the live session's way.
    #[arg(long, global = true, value_name = "PATH", env = "COMPASS_SOCKET")]
    pub socket: Option<PathBuf>,

    /// Increase log verbosity (repeatable). `RUST_LOG` overrides it.
    #[arg(short, long, global = true, action = clap::ArgAction::Count)]
    pub verbose: u8,

    /// The command to run.
    #[command(subcommand)]
    pub command: Command,
}

/// The URL schemes `vicinae <url>` takes as a deeplink, as the C++ URL
/// handler's desktop entry registers them.
pub const DEEPLINK_SCHEMES: [&str; 3] = ["vicinae", "raycast", "com.raycast"];

/// The command line with a bare deeplink (`vicinae raycast://oauth?…`, as a
/// desktop entry's `Exec=vicinae %u` runs it) turned into `vicinae deeplink
/// <url>`; anything else unchanged.
#[must_use]
pub fn with_deeplink(mut args: Vec<std::ffi::OsString>) -> Vec<std::ffi::OsString> {
    let is_deeplink = args.get(1).and_then(|arg| arg.to_str()).is_some_and(|arg| {
        arg.split_once(':')
            .is_some_and(|(scheme, _)| DEEPLINK_SCHEMES.contains(&scheme))
    });
    if is_deeplink {
        args.insert(1, "deeplink".into());
    }
    args
}

impl Cli {
    /// The socket path this invocation should use.
    #[must_use]
    pub fn socket_path(&self) -> SocketPath {
        match &self.socket {
            Some(path) => SocketPath::exact(path.clone()),
            None => SocketPath::from_env(),
        }
    }
}

/// Subcommands of `vicinae`.
#[derive(Debug, Subcommand, PartialEq, Eq)]
pub enum Command {
    /// Start a resident launcher, starting its engine if needed.
    /// Reopening activates the existing window. Escape hides it.
    Start {
        /// Wait for activation without opening a window. Does not enable autostart.
        #[arg(long)]
        hidden: bool,
    },

    /// Toggle the launcher window between shown and hidden.
    Toggle,

    /// Show the launcher window.
    #[command(alias = "open")]
    Show,

    /// Hide the launcher window.
    #[command(alias = "close")]
    Hide,

    /// Check that the engine is alive, printing its protocol version and pid.
    Ping,

    /// Run the engine.
    ///
    /// Serves the IPC socket until a `shutdown` request or a termination
    /// signal. It answers `ping`, `query` and `doctor` on any machine, display
    /// or not. `toggle`, `show` and `hide` are forwarded to a resident launcher
    /// window that attached over the same socket, and refused when none has --
    /// see ADR-0015.
    Serve {
        /// Do not bind the global launcher hotkey.
        ///
        /// The engine normally asks the GlobalShortcuts portal for
        /// `LOGO+space`, which on GNOME means a permission prompt. Pass this
        /// when your compositor already binds a key to `vicinae toggle`, or on
        /// a desktop with no GlobalShortcuts backend, and the engine will not
        /// ask. Everything else works exactly the same.
        #[arg(long)]
        no_hotkey: bool,
    },

    /// Ask a running engine to shut down.
    Shutdown,

    /// Search the running engine's index and print the ranked hits.
    Query {
        /// Search text. Joined with spaces if given as several words, so
        /// `vicinae query text editor` and `vicinae query "text editor"` agree.
        #[arg(required = true, num_args = 1..)]
        text: Vec<String>,

        /// Emit the hits as JSON.
        #[arg(long)]
        json: bool,

        /// Keep only hits from this provider, e.g. `applications` or
        /// `commands`. The C++ CLI's flag of the same name, so the parity
        /// harness can narrow both engines alike. Filters the engine's ranked
        /// list, so it can return fewer than `launcher.max_results`.
        #[arg(long, value_name = "PROVIDER")]
        provider: Option<String>,
    },

    /// Theme management (#153).
    #[command(subcommand)]
    Theme(ThemeCommand),

    /// The keyboard helper behind snippet keyword expansion.
    #[command(subcommand)]
    InputServer(InputServerCommand),

    /// Extension management.
    #[command(subcommand)]
    Ext(ExtCommand),

    /// The `vicinae.json` configuration: where it is, its schema, and migration.
    #[command(subcommand)]
    Config(ConfigCommand),

    /// One-off experiments that answer a question the code cannot.
    ///
    /// Hidden: these are addressed to whoever is answering the question — CI,
    /// or a person on a real machine — not to users, and each should be deleted
    /// or folded into a real subsystem once its question has an answer.
    #[command(hide = true, subcommand)]
    Spike(Spike),

    /// Hand a deeplink to the running engine.
    ///
    /// What the desktop runs for `raycast://`, `com.raycast:` and `vicinae://`
    /// URLs, and what a bare `vicinae <url>` becomes. It carries an OAuth
    /// provider's redirect (`raycast://oauth?code=…&state=…`) back to the
    /// extension that asked, and opens a store extension's detail page for
    /// `vicinae://extensions/<author>/<name>` (the Raycast store's for the
    /// `raycast://` spellings); other deeplinks are refused by name.
    Deeplink {
        /// The URL, verbatim.
        url: String,
    },

    /// Suite 1: run installed extensions headlessly and judge each first frame.
    ///
    /// Hidden: it is CI's, not a user's. Starts an engine of its own on a
    /// private socket, with this process's environment, so the caller chooses
    /// the extensions and the data directories. Exits non-zero when any
    /// command fails. See `crates/vicinae/src/conformance.rs`.
    #[command(hide = true)]
    Conformance {
        /// A JSON plan naming the commands and their inputs; without one, the
        /// first command of every installed extension.
        #[arg(long)]
        plan: Option<std::path::PathBuf>,

        /// Seconds each command has to draw a frame with something in it.
        #[arg(long, default_value_t = 30)]
        timeout: u64,

        /// Emit the report as JSON.
        #[arg(long)]
        json: bool,
    },

    /// Render a list view from stdin, and print the entry chosen.
    ///
    /// Shows the lines of standard input in the running launcher and waits;
    /// prints the chosen entry (or its index with `--format index`, or the
    /// search text when passed instead) and exits 0, or exits 1 when the list
    /// is dismissed.
    Dmenu(DmenuArgs),

    /// Open the launcher window.
    ///
    /// Attaches to an existing engine when available. Without one, indexes
    /// in-process and exits on dismissal. Use `start` for a resident session.
    Ui,

    /// Report what works on this machine and what does not.
    #[command(after_help = EXIT_CODE_HELP, after_long_help = EXIT_CODE_HELP)]
    Doctor {
        /// Print only the problems, and exit non-zero if any check failed.
        #[arg(long)]
        check_only: bool,

        /// Emit the report as JSON, for CI.
        #[arg(long)]
        json: bool,
    },
}

/// `vicinae dmenu`'s options, the C++ CLI's.
#[derive(Debug, Clone, PartialEq, Eq, clap::Args)]
pub struct DmenuArgs {
    /// Set the navigation title.
    #[arg(short = 'n', long)]
    pub navigation_title: Option<String>,
    /// Set the title of the main section. Use the {count} placeholder to
    /// render the current count.
    #[arg(short = 's', long)]
    pub section_title: Option<String>,
    /// Control the format of the output (data, index).
    #[arg(short = 'f', long, default_value = "data", value_parser = ["data", "index"], ignore_case = true)]
    pub format: String,
    /// Placeholder text to use in the search bar.
    #[arg(short = 'p', long)]
    pub placeholder: Option<String>,
    /// Initial search query.
    #[arg(short = 'q', long)]
    pub query: Option<String>,
    /// Window width in pixels.
    #[arg(short = 'W', long)]
    pub width: Option<u32>,
    /// Window height in pixels.
    #[arg(short = 'H', long)]
    pub height: Option<u32>,
    /// Do not insert a section heading.
    #[arg(long)]
    pub no_section: bool,
    /// Do not show quick look if available for a given entry.
    #[arg(long)]
    pub no_quick_look: bool,
    /// Do not show metadata section in quick look.
    #[arg(long)]
    pub no_metadata: bool,
    /// Hide the status bar footer.
    #[arg(long)]
    pub no_footer: bool,
}

/// Theme management subcommands (#153: Catppuccin, Dracula, Nord, Gruvbox, Tokyo Night, Solarized + System).
#[derive(Debug, Subcommand, PartialEq, Eq)]
pub enum ThemeCommand {
    /// List available themes.
    List {
        /// Emit the list as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Set the theme. Use `system` to return to OS natives.
    Set {
        /// Theme name (system, catppuccin, dracula, nord, gruvbox, tokyo-night, solarized).
        theme: String,
    },
    /// Reset to System (OS native) theme.
    Reset,
}

/// `vicinae input-server` subcommands.
#[derive(Debug, Subcommand, PartialEq, Eq)]
pub enum InputServerCommand {
    /// Whether it is on, running and able to type, and why not.
    Status {
        /// Emit the status as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Turn it on (`input_server.enabled`), starting it now if the engine runs.
    Enable,
    /// Turn it off; snippet keywords stop expanding.
    Disable,
}

/// Configuration subcommands.
#[derive(Debug, Subcommand, PartialEq, Eq)]
pub enum ConfigCommand {
    /// Print where `vicinae.json` and the C++ engine's `settings.json` are.
    Path,

    /// Print the JSON Schema for `vicinae.json`.
    ///
    /// The same document is published at `packaging/schema/vicinae.schema.json`.
    Schema,

    /// Translate the C++ engine's `settings.json` into `vicinae.json`.
    ///
    /// Without `--write` this only prints the result and what was and was not
    /// carried across. The C++ file is never modified.
    Migrate {
        /// The settings file to read. Defaults to the C++ engine's own.
        #[arg(long, value_name = "PATH")]
        from: Option<PathBuf>,

        /// Where to write. Defaults to this engine's `vicinae.json`.
        #[arg(long, value_name = "PATH")]
        to: Option<PathBuf>,

        /// Write the result instead of only printing it.
        #[arg(long)]
        write: bool,

        /// Replace an existing `vicinae.json`, keeping it as `vicinae.json.bak`.
        #[arg(long, requires = "write")]
        force: bool,

        /// Emit the migration report as JSON.
        #[arg(long)]
        json: bool,
    },
}

/// Extension management subcommands.
#[derive(Debug, Subcommand, PartialEq, Eq)]
pub enum ExtCommand {
    /// List available extensions and their status.
    List {
        /// Emit the list as JSON.
        #[arg(long)]
        json: bool,
    },
}

/// The spikes. See [`crate::spike`] for what each one is for.
#[derive(Debug, Subcommand, PartialEq, Eq)]
pub enum Spike {
    /// Spike A: bind a global shortcut through the portal and wait for it.
    ///
    /// Reports whether binding is permitted, whether the trigger granted is the
    /// one requested, and whether pressing the key reaches us. The third
    /// question is why this exists: it can only be answered by a real desktop
    /// with a real keypress, which is what the VM tier provides.
    GlobalShortcut {
        /// Preferred trigger, e.g. `SUPER+space`. Omit to express no
        /// preference, which is a different and also interesting answer.
        #[arg(long)]
        trigger: Option<String>,

        /// Shortcut id, echoed back on activation.
        #[arg(long, default_value = "compass.spike.toggle")]
        id: String,

        /// Seconds to wait for an activation after binding.
        #[arg(long, default_value_t = 60)]
        wait: u64,

        /// Emit the report as JSON, for CI.
        #[arg(long)]
        json: bool,
    },

    /// Spike B: can we sandbox a process inside an already-sandboxed Flatpak?
    ///
    /// Applies a Landlock ruleset and a seccomp filter to this process and
    /// tests whether each actually denies what it should while still allowing
    /// what it should. Phase 4's extension-host sandbox is designed on the
    /// assumption that both nest inside bubblewrap's; this measures it.
    ///
    /// Irreversible by nature: the process it runs in is confined afterwards,
    /// which is why it is its own command and not a doctor check.
    Sandbox {
        /// Emit the report as JSON, for CI.
        #[arg(long)]
        json: bool,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;
    use std::path::Path;

    fn parse(args: &[&str]) -> Cli {
        Cli::try_parse_from(args).expect("should parse")
    }

    #[test]
    fn the_command_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn a_bare_deeplink_becomes_the_deeplink_command() {
        let argv = |args: &[&str]| -> Vec<std::ffi::OsString> {
            args.iter().map(std::ffi::OsString::from).collect()
        };
        for url in [
            "raycast://oauth?code=c&state=s",
            "com.raycast:/oauth?code=c&state=s",
            "vicinae://extensions/x",
        ] {
            let parsed = Cli::try_parse_from(with_deeplink(argv(&["vicinae", url]))).expect(url);
            assert_eq!(parsed.command, Command::Deeplink { url: url.into() });
        }
        assert_eq!(
            with_deeplink(argv(&["vicinae", "toggle"])),
            argv(&["vicinae", "toggle"])
        );
        assert_eq!(
            with_deeplink(argv(&["vicinae", "https://example.com"])),
            argv(&["vicinae", "https://example.com"]),
            "a web URL is not ours to take"
        );
    }

    #[test]
    fn window_commands_parse() {
        assert_eq!(parse(&["vicinae", "toggle"]).command, Command::Toggle);
        assert_eq!(parse(&["vicinae", "show"]).command, Command::Show);
        assert_eq!(parse(&["vicinae", "hide"]).command, Command::Hide);
        assert_eq!(parse(&["vicinae", "ping"]).command, Command::Ping);
    }

    #[test]
    fn the_spike_parses_with_its_defaults() {
        // The defaults matter: the VM job passes only --trigger and --json, so
        // a changed default id here silently changes what the harness asserts.
        let Command::Spike(Spike::GlobalShortcut {
            trigger,
            id,
            wait,
            json,
        }) = parse(&["vicinae", "spike", "global-shortcut", "--json"]).command
        else {
            panic!("expected the global-shortcut spike");
        };
        assert_eq!(trigger, None);
        assert_eq!(id, "compass.spike.toggle");
        assert_eq!(wait, 60);
        assert!(json);
    }

    #[test]
    fn the_parity_harness_invocation_parses() {
        // crates/compass-testkit/src/parity.rs builds this exact argv, one call
        // per query, and it is the whole interface Suite 0 (§8.1) diffs the two
        // engines through. An argv assembled in one crate and parsed in another
        // has no compiler between the two, which is the same reason
        // `the_spike_parses_with_its_defaults` above exists.
        let cli = Cli::try_parse_from([
            "vicinae",
            "--socket",
            "/tmp/compass-parity/ipc.sock",
            "query",
            "--json",
            "firefox",
        ])
        .expect("the parity harness's argv must parse");

        let Command::Query { text, json, .. } = cli.command else {
            panic!("expected the query command");
        };
        assert_eq!(text, vec!["firefox".to_owned()]);
        assert!(
            json,
            "parity parses stdout as JSON, so --json must take effect"
        );

        // Two controls, because a test that only asserts the good case would
        // pass just as well against a CLI that accepts anything.

        // The order parity used before measurement: rejected, not tolerated.
        assert!(
            Cli::try_parse_from(["vicinae", "--json", "query", "firefox"]).is_err(),
            "a global --json would mean the original invocation was fine after all"
        );

        // And parity no longer passes --engine, because until the Phase 7
        // cutover the binary IS the engine: this one refuses `--engine cpp` at
        // runtime by design, and the C++ binary has no such flag to give.
        // Parsing is not the refusal — that happens later — so this only pins
        // that the flag still exists and still defaults to the Rust engine.
        let defaulted = Cli::try_parse_from(["vicinae", "query", "firefox"])
            .expect("query without --engine must parse");
        assert_eq!(defaulted.engine, Engine::Rust);
    }

    #[test]
    fn the_seeded_grant_names_the_spike_default_id() {
        // The VM image pre-seeds a GlobalShortcuts grant into dconf so that
        // `BindShortcuts` completes without a human at GNOME's consent dialog.
        // That bypass keys off the shortcut *id* alone: gnome-control-center
        // shows the dialog for any id it has not stored, so a rename here and
        // not there silently reinstates the dialog — and Spike A goes back to
        // hanging for its full timeout, which reads as a portal regression
        // rather than as a typo. Cheap to couple, expensive to debug.
        let seed = include_str!("../../../packaging/vmtest/compass-shortcuts.dconf");
        let Command::Spike(Spike::GlobalShortcut { id, .. }) =
            parse(&["vicinae", "spike", "global-shortcut"]).command
        else {
            panic!("expected the global-shortcut spike");
        };
        assert!(
            seed.contains(&format!("'{id}'")),
            "the seeded dconf grant does not mention the spike's default id {id:?}",
        );
    }

    #[test]
    fn the_sandbox_spike_parses() {
        let Command::Spike(Spike::Sandbox { json }) =
            parse(&["vicinae", "spike", "sandbox", "--json"]).command
        else {
            panic!("expected the sandbox spike");
        };
        assert!(json);
    }

    #[test]
    fn the_spike_is_hidden_but_reachable() {
        // Hidden from --help, yet it must still parse: a spike nobody can run
        // is worse than no spike, and `hide` is easy to confuse with disabling.
        let spike = Cli::command()
            .get_subcommands()
            .find(|c| c.get_name() == "spike")
            .expect("the spike subcommand exists")
            .clone();
        assert!(spike.is_hide_set(), "the spike should not appear in --help");
        assert!(Cli::try_parse_from(["vicinae", "spike", "global-shortcut"]).is_ok());
    }

    #[test]
    fn the_ui_command_parses() {
        assert_eq!(parse(&["vicinae", "ui"]).command, Command::Ui);
    }

    #[test]
    fn hidden_start_is_explicit() {
        assert_eq!(
            parse(&["vicinae", "start"]).command,
            Command::Start { hidden: false }
        );
        assert_eq!(
            parse(&["vicinae", "start", "--hidden"]).command,
            Command::Start { hidden: true }
        );
        assert!(Cli::try_parse_from(["vicinae", "ui", "--hidden"]).is_err());
    }

    #[test]
    fn the_engine_commands_parse() {
        assert_eq!(
            parse(&["vicinae", "serve"]).command,
            Command::Serve { no_hotkey: false }
        );
        assert_eq!(
            parse(&["vicinae", "serve", "--no-hotkey"]).command,
            Command::Serve { no_hotkey: true }
        );
        assert_eq!(parse(&["vicinae", "shutdown"]).command, Command::Shutdown);
    }

    #[test]
    fn a_multi_word_query_is_joined_rather_than_rejected() {
        // `vicinae query text editor` is what a person types; requiring the
        // quotes would be a papercut on the most-used command.
        let Command::Query { text, json, .. } =
            parse(&["vicinae", "query", "text", "editor"]).command
        else {
            panic!("expected a query");
        };
        assert_eq!(text, ["text", "editor"]);
        assert!(!json);
    }

    #[test]
    fn an_empty_query_is_a_usage_error() {
        // Distinct from `query ""`, which is the legitimate "show me everything"
        // case and must keep working.
        assert!(Cli::try_parse_from(["vicinae", "query"]).is_err());
        assert!(Cli::try_parse_from(["vicinae", "query", ""]).is_ok());
    }

    #[test]
    fn the_cpp_spellings_survive_as_aliases() {
        assert_eq!(parse(&["vicinae", "open"]).command, Command::Show);
        assert_eq!(parse(&["vicinae", "close"]).command, Command::Hide);
    }

    #[test]
    fn doctor_flags_default_off_and_parse_together() {
        assert_eq!(
            parse(&["vicinae", "doctor"]).command,
            Command::Doctor {
                check_only: false,
                json: false
            }
        );
        assert_eq!(
            parse(&["vicinae", "doctor", "--check-only", "--json"]).command,
            Command::Doctor {
                check_only: true,
                json: true
            }
        );
    }

    #[test]
    fn engine_defaults_to_rust_and_can_be_overridden() {
        assert_eq!(parse(&["vicinae", "ping"]).engine, Engine::Rust);
        assert_eq!(
            parse(&["vicinae", "--engine", "cpp", "ping"]).engine,
            Engine::Cpp
        );
    }

    #[test]
    fn an_unknown_engine_is_a_usage_error() {
        assert!(Cli::try_parse_from(["vicinae", "--engine", "go", "ping"]).is_err());
    }

    #[test]
    fn socket_override_is_used_verbatim() {
        let cli = parse(&["vicinae", "--socket", "/tmp/x.sock", "toggle"]);
        assert_eq!(cli.socket_path().as_path(), Path::new("/tmp/x.sock"));
        assert!(!cli.socket_path().is_fallback());
    }

    #[test]
    fn global_flags_may_follow_the_subcommand() {
        let cli = parse(&[
            "vicinae",
            "toggle",
            "--socket",
            "/tmp/y.sock",
            "--engine",
            "cpp",
        ]);
        assert_eq!(cli.command, Command::Toggle);
        assert_eq!(cli.engine, Engine::Cpp);
        assert_eq!(cli.socket_path().as_path(), Path::new("/tmp/y.sock"));
    }

    #[test]
    fn a_subcommand_is_required() {
        assert!(Cli::try_parse_from(["vicinae"]).is_err());
    }

    #[test]
    fn help_documents_the_exit_codes() {
        let help = Cli::command().render_long_help().to_string();
        assert!(help.contains("Exit codes:"));
        assert!(help.contains("no check failed"));

        let doctor_help = Cli::command()
            .find_subcommand_mut("doctor")
            .expect("doctor subcommand")
            .render_long_help()
            .to_string();
        assert!(doctor_help.contains("Exit codes:"));
    }

    #[test]
    fn ext_list_parses() {
        let cli = parse(&["vicinae", "ext", "list"]);
        assert_eq!(cli.command, Command::Ext(ExtCommand::List { json: false }));

        let cli = parse(&["vicinae", "ext", "list", "--json"]);
        assert_eq!(cli.command, Command::Ext(ExtCommand::List { json: true }));
    }

    #[test]
    fn theme_commands_parse() {
        assert_eq!(
            parse(&["vicinae", "theme", "list"]).command,
            Command::Theme(ThemeCommand::List { json: false })
        );
        assert_eq!(
            parse(&["vicinae", "theme", "set", "dracula"]).command,
            Command::Theme(ThemeCommand::Set {
                theme: "dracula".to_owned()
            })
        );
        assert_eq!(
            parse(&["vicinae", "theme", "reset"]).command,
            Command::Theme(ThemeCommand::Reset)
        );
    }
}
