//! The system-tray menu: what is in it, and what each entry does.
//!
//! A port of `TrayService` and `TrayServiceLinux`'s menu model
//! (`src/server/src/services/tray/`), minus the StatusNotifierItem D-Bus
//! plumbing.
//!
//! # The tray menu is the only way out when the launcher will not open
//!
//! Everything else in this program is reached by opening the launcher. When
//! that is the thing that is broken, the tray is what a person has left — so
//! the entries that matter are Toggle, Settings and Quit, and the rules about
//! when Quit appears are the ones worth getting right.

/// Where "Sponsor Vicinae" goes.
pub const SPONSOR_URL: &str = "https://github.com/sponsors/vicinaehq";
/// Where "Join the Discord" goes.
pub const DISCORD_URL: &str = "https://discord.gg/rP4ecD42p7";
/// Where "Follow on X" goes.
pub const FOLLOW_URL: &str = "https://x.com/aurelienb42";

/// The environment variable systemd sets for a unit it started.
///
/// Its presence is how the tray knows it is being supervised.
pub const SYSTEMD_INVOCATION_ENV: &str = "INVOCATION_ID";

/// The label on the entry that shows and hides the launcher.
pub const TOGGLE_LABEL: &str = "Toggle Vicinae";
/// The label on the About entry.
pub const ABOUT_LABEL: &str = "About Vicinae";
/// The label on the update-check entry.
pub const CHECK_FOR_UPDATES_LABEL: &str = "Check for Updates…";
/// The label on the settings entry.
pub const SETTINGS_LABEL: &str = "Settings…";
/// The macOS spelling of the settings entry.
pub const PREFERENCES_LABEL: &str = "Preferences…";
/// The label on the sponsor entry.
pub const SPONSOR_LABEL: &str = "Sponsor Vicinae";
/// The label on the Discord entry.
pub const DISCORD_LABEL: &str = "Join the Discord";
/// The label on the follow entry.
pub const FOLLOW_LABEL: &str = "Follow on X";
/// The label on the quit entry.
pub const QUIT_LABEL: &str = "Quit Vicinae";
/// The application's name, shown when no version is known.
pub const APP_NAME: &str = "Vicinae";

/// The label announcing an available update.
#[must_use]
pub fn update_available_label(tag: &str) -> String {
    format!("Update Available: {tag}")
}

/// An external page the menu can open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Link {
    /// The sponsorship page.
    Sponsor,
    /// The Discord invite.
    Discord,
    /// The author's profile.
    Follow,
}

impl Link {
    /// Where it goes.
    #[must_use]
    pub const fn url(self) -> &'static str {
        match self {
            Self::Sponsor => SPONSOR_URL,
            Self::Discord => DISCORD_URL,
            Self::Follow => FOLLOW_URL,
        }
    }
}

/// What a menu entry is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    /// Show or hide the launcher.
    Toggle,
    /// The version, shown and not clickable.
    Version,
    /// A dividing line.
    Separator,
    /// The About tab of settings.
    About,
    /// Settings.
    Settings,
    /// The sponsorship page.
    Sponsor,
    /// The Discord invite.
    Discord,
    /// The author's profile.
    Follow,
    /// Quit.
    Quit,
}

/// One entry, with the id D-Bus refers to it by.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MenuEntry {
    /// Its id.
    pub id: i32,
    /// What it is.
    pub kind: EntryKind,
}

/// What activating an entry asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Activation {
    /// Show or hide the launcher.
    Toggle,
    /// Open settings, at `tab` when there is one.
    OpenSettings {
        /// Which tab, or `None` for whatever it opens on.
        tab: Option<String>,
    },
    /// Open an external page.
    OpenLink(Link),
    /// Quit.
    Quit,
}

/// The entries of a tray menu, in order.
///
/// # Quit is absent under systemd
///
/// When `INVOCATION_ID` is set the process was started by a unit, and quitting
/// would either be undone by the restart policy or leave the unit stopped with
/// no obvious way to start it again. Neither is what somebody clicking Quit
/// wants, so the entry is simply not offered and `systemctl` is the way out.
#[must_use]
pub fn menu_entries(under_systemd: bool) -> Vec<MenuEntry> {
    let mut kinds = vec![
        EntryKind::Toggle,
        EntryKind::Version,
        EntryKind::Separator,
        EntryKind::About,
        EntryKind::Settings,
        EntryKind::Separator,
        EntryKind::Sponsor,
        EntryKind::Discord,
        EntryKind::Follow,
    ];
    if !under_systemd {
        kinds.push(EntryKind::Separator);
        kinds.push(EntryKind::Quit);
    }

    kinds
        .into_iter()
        .enumerate()
        .map(|(index, kind)| MenuEntry {
            // Ids start at one: zero is the menu's own root in the
            // StatusNotifierItem protocol, so an entry with that id would be
            // the menu itself.
            id: i32::try_from(index + 1).unwrap_or(i32::MAX),
            kind,
        })
        .collect()
}

/// The label an entry shows.
///
/// `version` is what `setVersion` was given; the Version entry reads just
/// `Vicinae` until it arrives, rather than showing an empty line.
#[must_use]
pub fn entry_label(kind: EntryKind, version: &str) -> String {
    match kind {
        EntryKind::Toggle => TOGGLE_LABEL.to_owned(),
        EntryKind::Version => {
            if version.is_empty() {
                APP_NAME.to_owned()
            } else {
                format!("{APP_NAME} {version}")
            }
        }
        EntryKind::About => ABOUT_LABEL.to_owned(),
        EntryKind::Settings => SETTINGS_LABEL.to_owned(),
        EntryKind::Sponsor => SPONSOR_LABEL.to_owned(),
        EntryKind::Discord => DISCORD_LABEL.to_owned(),
        EntryKind::Follow => FOLLOW_LABEL.to_owned(),
        EntryKind::Quit => QUIT_LABEL.to_owned(),
        EntryKind::Separator => String::new(),
    }
}

/// Whether an entry can be clicked.
///
/// Everything but the version, which is there to be read.
#[must_use]
pub fn entry_enabled(kind: EntryKind) -> bool {
    kind != EntryKind::Version
}

/// What clicking the entry with `id` asks for.
///
/// `None` for an id that is not in the menu, and for the two entries that do
/// nothing — a separator and the version. An unknown id is not an error: the
/// menu is rebuilt as the version and update state change, and a click can
/// arrive against the previous layout.
#[must_use]
pub fn activate(entries: &[MenuEntry], id: i32) -> Option<Activation> {
    let kind = entries.iter().find(|entry| entry.id == id)?.kind;
    match kind {
        EntryKind::Toggle => Some(Activation::Toggle),
        EntryKind::About => Some(Activation::OpenSettings {
            tab: Some("about".to_owned()),
        }),
        EntryKind::Settings => Some(Activation::OpenSettings { tab: None }),
        EntryKind::Sponsor => Some(Activation::OpenLink(Link::Sponsor)),
        EntryKind::Discord => Some(Activation::OpenLink(Link::Discord)),
        EntryKind::Follow => Some(Activation::OpenLink(Link::Follow)),
        EntryKind::Quit => Some(Activation::Quit),
        EntryKind::Version | EntryKind::Separator => None,
    }
}
