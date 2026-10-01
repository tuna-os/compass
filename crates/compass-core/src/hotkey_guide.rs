//! How someone opens the launcher from anywhere, in the words their desktop
//! uses.
//!
//! On GNOME and KDE, Compass asks the desktop's GlobalShortcuts portal to
//! bind its hotkey. Sway, Hyprland and niri have no such portal: the
//! compositor's own configuration binds keys, so the launcher's key is a line
//! there that runs `compass toggle`. Inside the Flatpak, that command is
//! `flatpak run org.tunaos.compass toggle`. Onboarding, the settings view,
//! `compass doctor` and the docs all say the same thing, from here.

use std::path::Path;

/// The file every Flatpak sandbox has at its root.
pub const FLATPAK_INFO_PATH: &str = "/.flatpak-info";

/// Whether this process runs inside the Flatpak.
#[must_use]
pub fn in_flatpak() -> bool {
    Path::new(FLATPAK_INFO_PATH).exists()
}

/// The command that shows or hides the launcher, as a user types it.
#[must_use]
pub fn toggle_command(flatpak: bool) -> String {
    if flatpak {
        format!("flatpak run {} toggle", compass_xdg::brand::APP_ID)
    } else {
        "compass toggle".to_owned()
    }
}

/// A compositor that binds keys in its own configuration rather than
/// through the GlobalShortcuts portal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compositor {
    /// Sway, found by `SWAYSOCK`.
    Sway,
    /// Hyprland, found by `HYPRLAND_INSTANCE_SIGNATURE`.
    Hyprland,
    /// niri, found by `NIRI_SOCKET`.
    Niri,
}

impl Compositor {
    /// Which one this session runs, from its environment: each compositor's
    /// own socket variable first, then `XDG_CURRENT_DESKTOP`.
    pub fn detect(mut var: impl FnMut(&str) -> Option<String>) -> Option<Self> {
        let set = |value: Option<String>| value.is_some_and(|value| !value.is_empty());
        if set(var("SWAYSOCK")) {
            return Some(Self::Sway);
        }
        if set(var("HYPRLAND_INSTANCE_SIGNATURE")) {
            return Some(Self::Hyprland);
        }
        if set(var("NIRI_SOCKET")) {
            return Some(Self::Niri);
        }
        let desktop = var("XDG_CURRENT_DESKTOP")?.to_ascii_lowercase();
        desktop.split(':').find_map(|name| match name {
            "sway" => Some(Self::Sway),
            "hyprland" => Some(Self::Hyprland),
            "niri" => Some(Self::Niri),
            _ => None,
        })
    }

    /// [`Compositor::detect`] over this process's environment.
    #[must_use]
    pub fn from_env() -> Option<Self> {
        Self::detect(|name| std::env::var(name).ok())
    }

    /// Its name, as its users write it.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Sway => "Sway",
            Self::Hyprland => "Hyprland",
            Self::Niri => "niri",
        }
    }

    /// Its configuration file, as a path under the home directory.
    #[must_use]
    pub const fn config_file(self) -> &'static str {
        match self {
            Self::Sway => "~/.config/sway/config",
            Self::Hyprland => "~/.config/hypr/hyprland.conf",
            Self::Niri => "~/.config/niri/config.kdl",
        }
    }

    /// The line that binds Super+Space to `command` in [`Self::config_file`].
    #[must_use]
    pub fn binding(self, command: &str) -> String {
        match self {
            Self::Sway => format!("bindsym $mod+space exec {command}"),
            Self::Hyprland => format!("bind = SUPER, SPACE, exec, {command}"),
            Self::Niri => {
                let words: Vec<String> = command
                    .split_whitespace()
                    .map(|word| format!("\"{word}\""))
                    .collect();
                format!("Mod+Space {{ spawn {}; }}", words.join(" "))
            }
        }
    }

    /// The whole instruction, as one sentence.
    #[must_use]
    pub fn instruction(self, command: &str) -> String {
        format!(
            "{} binds keys in its own configuration: add `{}` to {}, then reload it.",
            self.name(),
            self.binding(command),
            self.config_file()
        )
    }
}

/// What to tell someone whose desktop does not bind the hotkey for Compass:
/// the compositor's own line when it is one Compass knows, otherwise the
/// command to bind in the desktop's keyboard settings.
#[must_use]
pub fn bind_it_yourself(compositor: Option<Compositor>, flatpak: bool) -> String {
    let command = toggle_command(flatpak);
    match compositor {
        Some(compositor) => compositor.instruction(&command),
        None => format!(
            "Bind a key to `{command}` in your desktop's keyboard shortcut settings to open the launcher from anywhere."
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl FnMut(&str) -> Option<String> + 'a {
        move |name| {
            pairs
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| (*value).to_owned())
        }
    }

    #[test]
    fn each_compositor_is_found_by_its_own_socket() {
        assert_eq!(
            Compositor::detect(env(&[("SWAYSOCK", "/run/user/1000/sway-ipc.sock")])),
            Some(Compositor::Sway)
        );
        assert_eq!(
            Compositor::detect(env(&[("HYPRLAND_INSTANCE_SIGNATURE", "abc")])),
            Some(Compositor::Hyprland)
        );
        assert_eq!(
            Compositor::detect(env(&[("XDG_CURRENT_DESKTOP", "niri")])),
            Some(Compositor::Niri)
        );
        assert_eq!(
            Compositor::detect(env(&[("XDG_CURRENT_DESKTOP", "GNOME")])),
            None
        );
    }

    #[test]
    fn the_flatpak_command_is_the_one_its_users_can_run() {
        assert_eq!(toggle_command(false), "compass toggle");
        assert_eq!(
            toggle_command(true),
            "flatpak run org.tunaos.compass toggle"
        );
        assert_eq!(
            Compositor::Niri.binding(&toggle_command(true)),
            "Mod+Space { spawn \"flatpak\" \"run\" \"org.tunaos.compass\" \"toggle\"; }"
        );
        assert_eq!(
            bind_it_yourself(Some(Compositor::Sway), false),
            "Sway binds keys in its own configuration: add `bindsym $mod+space exec compass toggle` to ~/.config/sway/config, then reload it."
        );
    }
}
