//! Builtin commands: root-list rows that open a view instead of launching an
//! application.
//!
//! Each is a [`RootItem`] under the `commands`
//! provider, appended to the application index's roots so that one search
//! ranks both, with the same fuzzy scoring, typo tolerance and frecency, and
//! so the user's aliases and disabled flags apply to them unchanged.
//! `AppIndex::search_root` stays applications-only; `AppIndex::search_root_all`
//! returns both.

use crate::power_commands;
use crate::root_items::{RootItem, RootItemMeta, entrypoint_id};

/// Provider id of builtin commands.
pub const COMMANDS_PROVIDER_ID: &str = "commands";

/// A builtin command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BuiltinCommand {
    /// Which command this is.
    pub kind: CommandKind,
    /// Entrypoint within the `commands` provider, e.g. `clipboard-history`.
    pub entrypoint: &'static str,
    /// Row title.
    pub title: &'static str,
    /// Row subtitle.
    pub subtitle: &'static str,
    /// Extra search terms.
    pub keywords: &'static [&'static str],
    /// Builtin icon name (see `builtin_icon`).
    pub icon: &'static str,
}

/// What a command opens. Exhaustive, so adding one is a compile error at every
/// place that has to decide what to do with it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CommandKind {
    /// Browse and search clipboard history.
    ClipboardHistory,
    /// Focus an open window.
    SwitchWindows,
    /// Find an emoji or symbol and copy it.
    SearchEmojis,
    /// Search the file index and open a file.
    SearchFiles,
    /// The form a new quicklink is made in.
    CreateShortcut,
    /// List, open, edit and remove quicklinks.
    ManageShortcuts,
    /// The form a new snippet is made in.
    CreateSnippet,
    /// List, copy, paste, edit and remove snippets.
    ManageSnippets,
    /// Run a program on `PATH`, or a typed command line, in a terminal or
    /// directly.
    RunProgram,
    /// Browse the themes with a live preview, and keep one.
    SetTheme,
    /// Generate a new extension's boilerplate from a form.
    CreateExtension,
    /// Browse the installed fonts by script, and preview one.
    BrowseFonts,
    /// Browse, install and uninstall extensions from the Vicinae store.
    ExtensionStore,
    /// Browse, install and uninstall extensions from the Raycast store.
    RaycastStore,
    /// A Power Management command, by its id in [`crate::power_commands`].
    Power(&'static str),
    /// Browse and control the running media players.
    NowPlaying,
    /// Review and revoke what the user's Rhai scripts were allowed.
    ScriptPermissions,
    /// Browse, pin and remove past calculations.
    CalculatorHistory,
    /// A media command, by its id in [`crate::media_commands`].
    Media(&'static str),
    /// Browse every installed application, hidden ones included on request.
    BrowseApps,
    /// Choose the web browser links open in.
    SetDefaultBrowser,
    /// Choose the terminal commands run in.
    SetDefaultTerminal,
}

/// Every builtin command, in the order an empty query lists them. The power
/// commands index [`power_commands::COMMANDS`] in its registration order,
/// which a test pins.
pub const BUILTIN_COMMANDS: &[BuiltinCommand] = &[
    BuiltinCommand {
        kind: CommandKind::ClipboardHistory,
        entrypoint: "clipboard-history",
        title: "Clipboard History",
        subtitle: "Search what you have copied",
        keywords: &["clipboard", "copy", "paste", "history", "pasteboard"],
        icon: "copy-clipboard",
    },
    BuiltinCommand {
        kind: CommandKind::SwitchWindows,
        entrypoint: "switch-windows",
        title: "Switch Windows",
        subtitle: "Focus an open window",
        keywords: &[
            "windows", "window", "switch", "focus", "alt tab", "switcher",
        ],
        icon: "switch-windows",
    },
    BuiltinCommand {
        kind: CommandKind::SearchEmojis,
        entrypoint: "search-emojis",
        title: "Search Emojis & Symbols",
        subtitle: "Find a character and copy it",
        keywords: &[
            "emoji",
            "emojis",
            "symbol",
            "symbols",
            "glyph",
            "character",
            "unicode",
        ],
        icon: "emoji",
    },
    BuiltinCommand {
        kind: CommandKind::SearchFiles,
        entrypoint: "search-files",
        title: "Search Files",
        subtitle: "Search files on your system",
        keywords: &["files", "file", "find", "documents", "folders", "open"],
        icon: "magnifying-glass",
    },
    BuiltinCommand {
        kind: CommandKind::CreateShortcut,
        entrypoint: "create-shortcut",
        title: "Create Shortcut",
        subtitle: "Save a link with placeholders",
        keywords: &["shortcut", "quicklink", "link", "bookmark", "url", "new"],
        icon: "bolt",
    },
    BuiltinCommand {
        kind: CommandKind::ManageShortcuts,
        entrypoint: "manage-shortcuts",
        title: "Manage Shortcuts",
        subtitle: "Open, edit and remove your shortcuts",
        keywords: &["shortcuts", "quicklinks", "links", "bookmarks"],
        icon: "bolt",
    },
    BuiltinCommand {
        kind: CommandKind::CreateSnippet,
        entrypoint: "create-snippet",
        title: "Create Snippet",
        subtitle: "Save text to paste or expand as you type",
        keywords: &["snippet", "text", "template", "expansion", "new"],
        icon: "snippets",
    },
    BuiltinCommand {
        kind: CommandKind::ManageSnippets,
        entrypoint: "manage-snippets",
        title: "Manage Snippets",
        subtitle: "Copy, paste, edit and remove your snippets",
        keywords: &["snippets", "text", "templates", "paste", "search"],
        icon: "snippets",
    },
    BuiltinCommand {
        kind: CommandKind::RunProgram,
        entrypoint: "run-program",
        title: "Run Terminal Program",
        subtitle: "Run a program in a terminal window",
        keywords: &["shell command", "run program", "terminal", "execute"],
        icon: "terminal",
    },
    BuiltinCommand {
        kind: CommandKind::SetTheme,
        entrypoint: "set-theme",
        title: "Set Theme",
        subtitle: "Browse the themes and pick one",
        keywords: &["theme", "themes", "colors", "appearance", "dark", "light"],
        icon: "brush",
    },
    BuiltinCommand {
        kind: CommandKind::CreateExtension,
        entrypoint: "create-extension",
        title: "Create Extension",
        subtitle: "Start a new extension from a template",
        keywords: &["developer", "extension", "boilerplate", "template", "new"],
        icon: "hammer",
    },
    BuiltinCommand {
        kind: CommandKind::BrowseFonts,
        entrypoint: "browse-fonts",
        title: "Browse Fonts",
        subtitle: "Browse and preview the installed fonts",
        keywords: &["fonts", "font", "typeface", "typography", "specimen"],
        icon: "text",
    },
    BuiltinCommand {
        kind: CommandKind::ExtensionStore,
        entrypoint: "store",
        title: "Extension Store",
        subtitle: "Install extensions from the Vicinae store",
        keywords: &["store", "extensions", "install", "vicinae", "plugins"],
        icon: "cart",
    },
    BuiltinCommand {
        kind: CommandKind::RaycastStore,
        entrypoint: "raycast-store",
        title: "Raycast Store",
        subtitle: "Install compatible extensions from the Raycast store",
        keywords: &["store", "extensions", "install", "raycast", "plugins"],
        icon: "raycast",
    },
    BuiltinCommand {
        kind: CommandKind::Power(power_commands::COMMANDS[0].id),
        entrypoint: power_commands::COMMANDS[0].id,
        title: power_commands::COMMANDS[0].name,
        subtitle: power_commands::COMMANDS[0].description,
        keywords: power_commands::COMMANDS[0].keywords,
        icon: "power",
    },
    BuiltinCommand {
        kind: CommandKind::Power(power_commands::COMMANDS[1].id),
        entrypoint: power_commands::COMMANDS[1].id,
        title: power_commands::COMMANDS[1].name,
        subtitle: power_commands::COMMANDS[1].description,
        keywords: power_commands::COMMANDS[1].keywords,
        icon: "rotate-clockwise",
    },
    BuiltinCommand {
        kind: CommandKind::Power(power_commands::COMMANDS[2].id),
        entrypoint: power_commands::COMMANDS[2].id,
        title: power_commands::COMMANDS[2].name,
        subtitle: power_commands::COMMANDS[2].description,
        keywords: power_commands::COMMANDS[2].keywords,
        icon: "moon",
    },
    BuiltinCommand {
        kind: CommandKind::Power(power_commands::COMMANDS[3].id),
        entrypoint: power_commands::COMMANDS[3].id,
        title: power_commands::COMMANDS[3].name,
        subtitle: power_commands::COMMANDS[3].description,
        keywords: power_commands::COMMANDS[3].keywords,
        icon: "lock",
    },
    BuiltinCommand {
        kind: CommandKind::Power(power_commands::COMMANDS[4].id),
        entrypoint: power_commands::COMMANDS[4].id,
        title: power_commands::COMMANDS[4].name,
        subtitle: power_commands::COMMANDS[4].description,
        keywords: power_commands::COMMANDS[4].keywords,
        icon: "logout",
    },
    BuiltinCommand {
        kind: CommandKind::Power(power_commands::COMMANDS[5].id),
        entrypoint: power_commands::COMMANDS[5].id,
        title: power_commands::COMMANDS[5].name,
        subtitle: power_commands::COMMANDS[5].description,
        keywords: power_commands::COMMANDS[5].keywords,
        icon: "moon",
    },
    BuiltinCommand {
        kind: CommandKind::Power(power_commands::COMMANDS[6].id),
        entrypoint: power_commands::COMMANDS[6].id,
        title: power_commands::COMMANDS[6].name,
        subtitle: power_commands::COMMANDS[6].description,
        keywords: power_commands::COMMANDS[6].keywords,
        icon: "moon",
    },
    BuiltinCommand {
        kind: CommandKind::Power(power_commands::COMMANDS[7].id),
        entrypoint: power_commands::COMMANDS[7].id,
        title: power_commands::COMMANDS[7].name,
        subtitle: power_commands::COMMANDS[7].description,
        keywords: power_commands::COMMANDS[7].keywords,
        icon: "rotate-clockwise",
    },
    BuiltinCommand {
        kind: CommandKind::ScriptPermissions,
        entrypoint: "script-permissions",
        title: "Script Permissions",
        subtitle: "Review and revoke what your Rhai scripts may do",
        keywords: &[
            "rhai",
            "scripts",
            "permissions",
            "consent",
            "grants",
            "revoke",
        ],
        icon: "key",
    },
    BuiltinCommand {
        kind: CommandKind::CalculatorHistory,
        entrypoint: "calculator-history",
        title: "Calculator History",
        subtitle: "Browse past calculations",
        keywords: &["calculator", "history", "calc", "math", "calculations"],
        icon: "calculator",
    },
    BuiltinCommand {
        kind: CommandKind::NowPlaying,
        entrypoint: "now-playing",
        title: "Now Playing",
        subtitle: "Browse and control running media players",
        keywords: &["media", "music", "player", "mpris"],
        icon: "music",
    },
    BuiltinCommand {
        kind: CommandKind::Media("play-pause"),
        entrypoint: "play-pause",
        title: "Play / Pause",
        subtitle: "Toggle playback of the active media player",
        keywords: &["media", "music", "play", "pause", "resume"],
        icon: "play",
    },
    BuiltinCommand {
        kind: CommandKind::Media("next-track"),
        entrypoint: "next-track",
        title: "Next Track",
        subtitle: "Skip to the next track",
        keywords: &["media", "music", "skip", "forward"],
        icon: "forward",
    },
    BuiltinCommand {
        kind: CommandKind::Media("previous-track"),
        entrypoint: "previous-track",
        title: "Previous Track",
        subtitle: "Skip to the previous track",
        keywords: &["media", "music", "back", "rewind"],
        icon: "rewind",
    },
    BuiltinCommand {
        kind: CommandKind::Media("volume-up"),
        entrypoint: "volume-up",
        title: "Turn Volume Up",
        subtitle: "Increase system volume",
        keywords: &["audio", "sound", "louder"],
        icon: "speaker-up",
    },
    BuiltinCommand {
        kind: CommandKind::Media("volume-down"),
        entrypoint: "volume-down",
        title: "Turn Volume Down",
        subtitle: "Decrease system volume",
        keywords: &["audio", "sound", "quieter"],
        icon: "speaker-down",
    },
    BuiltinCommand {
        kind: CommandKind::Media("volume-100"),
        entrypoint: "volume-100",
        title: "Set Volume to 100%",
        subtitle: "Set system volume to 100%",
        keywords: &["audio", "sound", "volume"],
        icon: "speaker-high",
    },
    BuiltinCommand {
        kind: CommandKind::Media("volume-75"),
        entrypoint: "volume-75",
        title: "Set Volume to 75%",
        subtitle: "Set system volume to 75%",
        keywords: &["audio", "sound", "volume"],
        icon: "speaker-high",
    },
    BuiltinCommand {
        kind: CommandKind::Media("volume-50"),
        entrypoint: "volume-50",
        title: "Set Volume to 50%",
        subtitle: "Set system volume to 50%",
        keywords: &["audio", "sound", "volume"],
        icon: "speaker-low",
    },
    BuiltinCommand {
        kind: CommandKind::Media("volume-25"),
        entrypoint: "volume-25",
        title: "Set Volume to 25%",
        subtitle: "Set system volume to 25%",
        keywords: &["audio", "sound", "volume"],
        icon: "speaker-low",
    },
    BuiltinCommand {
        kind: CommandKind::Media("volume-0"),
        entrypoint: "volume-0",
        title: "Set Volume to 0%",
        subtitle: "Set system volume to 0%",
        keywords: &["audio", "sound", "volume"],
        icon: "speaker-off",
    },
    BuiltinCommand {
        kind: CommandKind::Media("toggle-mute"),
        entrypoint: "toggle-mute",
        title: "Toggle Mute",
        subtitle: "Mute or unmute system audio",
        keywords: &["audio", "sound", "volume", "mute", "unmute"],
        icon: "speaker-off",
    },
    // The system extension's other three (`system-extension.hpp`). Browse
    // Apps is `isDefaultDisabled`: see `BuiltinCommand::default_disabled`.
    BuiltinCommand {
        kind: CommandKind::BrowseApps,
        entrypoint: "browse-apps",
        title: "Browse Apps",
        subtitle: "Browse all applications that are installed on the system",
        keywords: &[],
        icon: "box",
    },
    BuiltinCommand {
        kind: CommandKind::SetDefaultTerminal,
        entrypoint: "set-default-terminal",
        title: "Set Default Terminal",
        subtitle: "Change the default system terminal",
        keywords: &[],
        icon: "terminal",
    },
    BuiltinCommand {
        kind: CommandKind::SetDefaultBrowser,
        entrypoint: "set-default-browser",
        title: "Set Default Browser",
        subtitle: "Change the default system web browser",
        keywords: &[],
        icon: "globe-01",
    },
];

impl BuiltinCommand {
    /// The `commands:<entrypoint>` id that addresses it in root search, on the
    /// wire, and as its frecency key.
    #[must_use]
    pub fn id(&self) -> String {
        entrypoint_id(COMMANDS_PROVIDER_ID, self.entrypoint)
    }

    /// Whether the command is left out of the root list until the user
    /// enables it: `isDefaultDisabled`, which only Browse Apps sets.
    #[must_use]
    pub fn default_disabled(&self) -> bool {
        self.kind == CommandKind::BrowseApps
    }

    /// Its root-list row.
    #[must_use]
    pub fn root_item(&self) -> RootItem {
        RootItem {
            id: self.id(),
            title: self.title.to_owned(),
            unlocalized_title: None,
            subtitle: self.subtitle.to_owned(),
            keywords: self.keywords.iter().map(|&k| k.to_owned()).collect(),
            meta: RootItemMeta {
                provider_id: COMMANDS_PROVIDER_ID.to_owned(),
                enabled: !self.default_disabled(),
                ..RootItemMeta::default()
            },
        }
    }
}

/// The command a `commands:<entrypoint>` id names.
#[must_use]
pub fn by_id(id: &str) -> Option<&'static BuiltinCommand> {
    BUILTIN_COMMANDS.iter().find(|command| command.id() == id)
}

/// The C++ id of Search Files, which the default `fallbacks` list names.
pub const SEARCH_FILES_FALLBACK_ID: &str = "files:search";

/// The builtin command a `fallbacks` entry names, when it is one that can be
/// a fallback (`isFallback`): Search Files, by its C++ id or its Compass one.
#[must_use]
pub fn fallback(id: &str) -> Option<&'static BuiltinCommand> {
    let command = if id == SEARCH_FILES_FALLBACK_ID {
        BUILTIN_COMMANDS
            .iter()
            .find(|command| command.kind == CommandKind::SearchFiles)
    } else {
        by_id(id)
    }?;
    (command.kind == CommandKind::SearchFiles).then_some(command)
}

/// The C++ ids of the builtin commands a configuration is likely to name,
/// with the command each is here. The C++ addresses a builtin as
/// `<extension>:<command>` (`clipboard:history`), Compass as
/// `commands:<entrypoint>`; the default `favorites` list and a file written
/// by the C++ use the former.
pub const CPP_BUILTIN_IDS: &[(&str, CommandKind)] = &[
    ("clipboard:history", CommandKind::ClipboardHistory),
    ("files:search", CommandKind::SearchFiles),
    ("core:search-emojis", CommandKind::SearchEmojis),
];

/// The id Compass knows an entrypoint by: a C++ builtin's id becomes its
/// command's `commands:` id, and anything else is returned as it is.
#[must_use]
pub fn canonical_id(id: &str) -> String {
    CPP_BUILTIN_IDS
        .iter()
        .find(|(cpp, _)| *cpp == id)
        .and_then(|(_, kind)| {
            BUILTIN_COMMANDS
                .iter()
                .find(|command| command.kind == *kind)
        })
        .map_or_else(|| id.to_owned(), BuiltinCommand::id)
}

/// Whether running the command opens a view in the launcher, which is what
/// lets its alias and a space open it (`supportsAliasSpaceShortcut`, which
/// the C++ answers with `isView()`): a command that runs and hides has
/// nothing to show for it.
#[must_use]
pub const fn opens_a_view(kind: CommandKind) -> bool {
    !matches!(kind, CommandKind::Power(_) | CommandKind::Media(_))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cpp_builtin_id_names_its_compass_command_and_others_are_kept() {
        assert_eq!(
            canonical_id("clipboard:history"),
            "commands:clipboard-history"
        );
        assert_eq!(canonical_id("core:search-emojis"), "commands:search-emojis");
        assert_eq!(canonical_id("applications:firefox"), "applications:firefox");
        for (_, kind) in CPP_BUILTIN_IDS {
            assert!(BUILTIN_COMMANDS.iter().any(|command| command.kind == *kind));
        }
    }

    #[test]
    fn the_power_commands_are_the_power_catalogue_in_order() {
        let power: Vec<&str> = BUILTIN_COMMANDS
            .iter()
            .filter_map(|c| match c.kind {
                CommandKind::Power(id) => Some(id),
                _ => None,
            })
            .collect();
        let catalogue: Vec<&str> = power_commands::COMMANDS.iter().map(|c| c.id).collect();
        assert_eq!(power, catalogue);
    }

    #[test]
    fn the_media_commands_are_ones_the_media_extension_registers() {
        let registered = crate::media_commands::registered_commands(true);
        for command in BUILTIN_COMMANDS {
            if let CommandKind::Media(id) = command.kind {
                assert_eq!(id, command.entrypoint);
                assert!(registered.iter().any(|r| r == id), "{id}");
            }
        }
    }

    #[test]
    fn search_files_is_the_one_fallback_by_either_id() {
        assert_eq!(
            fallback("files:search").map(|c| c.kind),
            Some(CommandKind::SearchFiles)
        );
        assert_eq!(
            fallback("commands:search-files").map(|c| c.kind),
            Some(CommandKind::SearchFiles)
        );
        assert_eq!(fallback("commands:clipboard-history"), None);
        assert_eq!(fallback("nothing:here"), None);
    }

    #[test]
    fn ids_are_unique_and_round_trip() {
        let mut ids: Vec<String> = BUILTIN_COMMANDS.iter().map(BuiltinCommand::id).collect();
        for id in &ids {
            assert_eq!(
                by_id(id).map(BuiltinCommand::id).as_deref(),
                Some(id.as_str())
            );
        }
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), BUILTIN_COMMANDS.len());
        assert_eq!(
            by_id("commands:clipboard-history").map(|c| c.kind),
            Some(CommandKind::ClipboardHistory)
        );
        assert!(by_id("applications:clipboard-history").is_none());
    }

    #[test]
    fn every_icon_is_a_real_builtin_icon() {
        for command in BUILTIN_COMMANDS {
            assert!(
                crate::builtin_icon::is_builtin(command.icon),
                "{} names an icon that does not exist: {}",
                command.title,
                command.icon
            );
        }
    }
}
