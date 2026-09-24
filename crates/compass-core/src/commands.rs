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
    /// A Power Management command, by its id in [`crate::power_commands`].
    Power(&'static str),
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
];

impl BuiltinCommand {
    /// The `commands:<entrypoint>` id that addresses it in root search, on the
    /// wire, and as its frecency key.
    #[must_use]
    pub fn id(&self) -> String {
        entrypoint_id(COMMANDS_PROVIDER_ID, self.entrypoint)
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
                enabled: true,
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

#[cfg(test)]
mod tests {
    use super::*;

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
