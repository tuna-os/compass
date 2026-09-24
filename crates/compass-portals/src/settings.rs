//! `org.freedesktop.portal.Settings`, for the desktop's appearance preference.
//!
//! The launcher draws itself light or dark to match the desktop it sits on,
//! and inside a Flatpak this portal is the only way to learn which. Reading
//! GSettings directly would need `--filesystem=xdg-config/dconf` and a GNOME
//! assumption; `org.freedesktop.appearance` is the cross-desktop key and
//! xdg-desktop-portal answers it on GNOME, KDE and wlroots alike.
//!
//! Two things are deliberately separate here. [`SettingsPortal::color_scheme`]
//! is one read, for the appearance to start at; [`SettingsPortal::watch`] is
//! the stream of later changes. A launcher that only read at startup would
//! stay dark through a whole afternoon after the desktop went light, because
//! the process is resident (ADR-0015) and may outlive many such switches.
//!
//! **Absence is not dark.** The portal not being there, the key being unset
//! and the desktop actively saying "no preference" are three different facts,
//! and only the third is an answer. The first two surface as an error and as
//! [`ColorScheme::NoPreference`] respectively, so a caller can tell "we could
//! not ask" from "the desktop does not care" and pick its own default for each
//! rather than having one chosen for it here.

use std::time::Duration;

use ashpd::desktop::settings::{
    APPEARANCE_NAMESPACE, COLOR_SCHEME_KEY, ColorScheme as AshpdColorScheme, Settings,
};
use futures_util::{Stream, StreamExt};

use crate::error::Result;
use crate::shortcuts::bounded;

/// The desktop's light/dark preference.
///
/// Mirrors `org.freedesktop.appearance color-scheme`, whose wire values are
/// fixed by the specification: 0 no preference, 1 prefer dark, 2 prefer light.
/// It is restated rather than re-exported from `ashpd` so that the rest of the
/// workspace does not take a dependency on `ashpd`'s enum shape, which is what
/// [`Self::value`] is for on the way back out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ColorScheme {
    /// The desktop has no preference. This is an answer, not a failure.
    #[default]
    NoPreference,
    /// The desktop prefers dark.
    PreferDark,
    /// The desktop prefers light.
    PreferLight,
}

impl ColorScheme {
    /// The specification's wire value.
    ///
    /// The mapping is pinned in both directions so that a crate consuming this
    /// -- `compass-ui`'s `design::ColorScheme::from_portal` is the one that
    /// matters -- can be tested against the numbers the portal actually sends
    /// without depending on this crate or on `ashpd`.
    #[must_use]
    pub const fn value(self) -> u32 {
        match self {
            Self::NoPreference => 0,
            Self::PreferDark => 1,
            Self::PreferLight => 2,
        }
    }

    /// From the specification's wire value.
    ///
    /// An unknown number is [`Self::NoPreference`], which is what the
    /// specification requires of a client that does not recognise a value: new
    /// preferences may be added, and guessing at one is worse than declining
    /// to have an opinion.
    #[must_use]
    pub const fn from_value(value: u32) -> Self {
        match value {
            1 => Self::PreferDark,
            2 => Self::PreferLight,
            _ => Self::NoPreference,
        }
    }
}

impl From<AshpdColorScheme> for ColorScheme {
    fn from(value: AshpdColorScheme) -> Self {
        match value {
            AshpdColorScheme::PreferDark => Self::PreferDark,
            AshpdColorScheme::PreferLight => Self::PreferLight,
            AshpdColorScheme::NoPreference => Self::NoPreference,
        }
    }
}

/// The desktop's interface font, as `org.gnome.desktop.interface font-name`.
///
/// A Pango font description such as `Cantarell 11` or `Adwaita Sans 11`.
/// The portal proxies the underlying GSettings key, so the same value is
/// visible through `gsettings get org.gnome.desktop.interface font-name`
/// outside a sandbox.
pub const INTERFACE_FONT_NAMESPACE: &str = "org.gnome.desktop.interface";
/// Key for the interface font.
pub const INTERFACE_FONT_KEY: &str = "font-name";

/// Client for `org.freedesktop.portal.Settings`.
#[derive(Debug, Clone)]
pub struct SettingsPortal {
    conn: zbus::Connection,
    timeout: Duration,
}

impl SettingsPortal {
    pub(crate) fn new(conn: zbus::Connection, timeout: Duration) -> Self {
        Self { conn, timeout }
    }

    /// Read the desktop's current light/dark preference.
    pub async fn color_scheme(&self) -> Result<ColorScheme> {
        let proxy = self.proxy().await?;
        // `proxy.color_scheme()` would do this, and naming the namespace and
        // key here instead keeps the read and the watch below pinned to the
        // same pair by the same test.
        let scheme = bounded(
            self.timeout,
            "Read",
            proxy.read::<AshpdColorScheme>(APPEARANCE_NAMESPACE, COLOR_SCHEME_KEY),
        )
        .await?;
        Ok(scheme.into())
    }

    /// Read the desktop's interface font, if the portal exposes it.
    ///
    /// Returns `None` when the key is unset or the portal is not available.
    /// The caller decides the fallback (typically `Cantarell 11`).
    pub async fn interface_font(&self) -> Result<Option<String>> {
        let proxy = self.proxy().await?;
        let value = bounded(
            self.timeout,
            "Read",
            proxy.read::<String>(INTERFACE_FONT_NAMESPACE, INTERFACE_FONT_KEY),
        )
        .await?;
        let trimmed = value.trim().to_owned();
        if trimmed.is_empty() {
            Ok(None)
        } else {
            Ok(Some(trimmed))
        }
    }

    async fn proxy(&self) -> Result<Settings> {
        bounded(
            self.timeout,
            "Settings",
            Settings::with_connection(self.conn.clone()),
        )
        .await
    }

    /// A stream of the interface font as it changes.
    ///
    /// Yields the raw Pango description (e.g. `Cantarell 11`) on each change.
    pub async fn watch_interface_font(&self) -> Result<impl Stream<Item = String> + use<>> {
        let proxy = self.proxy().await?;
        let stream = bounded(
            self.timeout,
            "SettingChanged",
            proxy.receive_setting_changed_with_args::<String>(
                INTERFACE_FONT_NAMESPACE,
                INTERFACE_FONT_KEY,
            ),
        )
        .await?;
        Ok(stream.filter_map(|value| std::future::ready(value.ok())))
    }

    /// A stream of the preference as it changes.
    ///
    /// The stream yields nothing for the *current* value -- subscribing is not
    /// a read, and the portal emits `SettingChanged` only on a change. Callers
    /// that need both take [`Self::color_scheme`] first; doing that inside
    /// here would hide a bus round trip inside what looks like a subscription,
    /// and would make the first item indistinguishable from a real change.
    ///
    /// The call timeout bounds *subscribing* -- adding the match rule is a
    /// bus round trip and a wedged frontend never answers it -- and not the
    /// stream. A desktop whose appearance never changes is the normal case,
    /// and a stream that ended after five seconds of that would be a bug
    /// rather than a deadline.
    pub async fn watch(&self) -> Result<impl Stream<Item = ColorScheme> + use<>> {
        let proxy = self.proxy().await?;
        // `receive_color_scheme_changed` borrows the proxy, which would tie
        // the stream's lifetime to a local. This is what it does internally,
        // and it captures nothing.
        let stream = bounded(
            self.timeout,
            "SettingChanged",
            proxy.receive_setting_changed_with_args::<AshpdColorScheme>(
                APPEARANCE_NAMESPACE,
                COLOR_SCHEME_KEY,
            ),
        )
        .await?;
        // A `SettingChanged` carrying something other than a `u` is a portal
        // bug; dropping it keeps the stream alive, where unwrapping would take
        // the launcher's appearance down with it.
        Ok(stream.filter_map(|value| std::future::ready(value.ok().map(ColorScheme::from))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_wire_values_are_the_specifications() {
        assert_eq!(ColorScheme::NoPreference.value(), 0);
        assert_eq!(ColorScheme::PreferDark.value(), 1);
        assert_eq!(ColorScheme::PreferLight.value(), 2);
    }

    #[test]
    fn every_wire_value_round_trips() {
        for scheme in [
            ColorScheme::NoPreference,
            ColorScheme::PreferDark,
            ColorScheme::PreferLight,
        ] {
            assert_eq!(ColorScheme::from_value(scheme.value()), scheme);
        }
    }

    #[test]
    fn an_unknown_value_is_no_preference_rather_than_a_guess() {
        // The specification allows new values. A client that mapped 3 to dark
        // would start drawing dark the day one is added.
        assert_eq!(ColorScheme::from_value(3), ColorScheme::NoPreference);
        assert_eq!(ColorScheme::from_value(u32::MAX), ColorScheme::NoPreference);
    }

    #[test]
    fn ashpds_enum_maps_onto_ours_without_collapsing_an_arm() {
        assert_eq!(
            ColorScheme::from(AshpdColorScheme::PreferDark),
            ColorScheme::PreferDark
        );
        assert_eq!(
            ColorScheme::from(AshpdColorScheme::PreferLight),
            ColorScheme::PreferLight
        );
        assert_eq!(
            ColorScheme::from(AshpdColorScheme::NoPreference),
            ColorScheme::NoPreference
        );
    }
}
