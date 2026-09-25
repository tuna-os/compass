//! Feeding the launcher window the desktop's light/dark preference.
//!
//! This is the one place that knows the preference comes from a portal.
//! `compass-ui` takes a channel and nothing else (ADR-0013), and
//! `compass-portals` knows nothing about windows; the join is here.
//!
//! # Why this does not block the launcher's start
//!
//! The first read is a bus round trip, and the launcher's cold start is
//! already the slow path. So the thread below is given a short budget to
//! answer -- long enough for a portal that is there, short enough that a
//! wedged one costs a quarter of a second rather than the call timeout -- and
//! whatever it has not answered by then arrives later through the same channel
//! as any real change. The cost of being wrong is one frame in the other
//! appearance, not a launcher that will not open.

use std::sync::mpsc;
use std::time::Duration;

use compass_ui::appearance::AppearanceLink;
use compass_ui::design::{Appearance, ColorScheme};

/// A persisted colour choice for the launcher.
///
/// `System` is the default and is the only mode that subscribes to portal
/// changes. Fixed modes intentionally do not keep a portal watcher alive: an
/// explicit choice must remain explicit when the desktop changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ColorMode {
    /// Follow the desktop's native light/dark preference.
    System,
    /// Always use the light palette.
    Light,
    /// Always use the dark palette.
    Dark,
}

impl ColorMode {
    /// Parse the durable config spelling, treating unknown values as System.
    pub(crate) fn from_config(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "light" => Self::Light,
            "dark" => Self::Dark,
            _ => Self::System,
        }
    }

    /// Whether the durable spelling is one this build understands.
    pub(crate) fn is_known(value: &str) -> bool {
        matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "system" | "light" | "dark"
        )
    }

    /// The fixed appearance, if this mode does not follow the system.
    fn fixed_appearance(self) -> Option<Appearance> {
        match self {
            Self::System => None,
            Self::Light => Some(Appearance::Light),
            Self::Dark => Some(Appearance::Dark),
        }
    }
}

/// How long the window waits for the first read before drawing anyway.
const FIRST_READ_BUDGET: Duration = Duration::from_millis(250);

/// The appearance to draw with when nothing has said otherwise.
///
/// Light because that is Adwaita's no-preference default. A machine without a
/// Settings portal still gets a readable, documented native fallback.
const FALLBACK: Appearance = Appearance::Light;

/// Start in the configured colour mode.
///
/// Returns what to draw the first frame with, and (for System mode) the link
/// later changes arrive on. Fixed modes return no link, which keeps an
/// explicit choice from being overwritten by a desktop change.
pub(crate) fn follow(mode: ColorMode) -> (Appearance, Option<AppearanceLink>) {
    if let Some(appearance) = mode.fixed_appearance() {
        return (appearance, None);
    }

    let (link, sender) = AppearanceLink::new();
    let (first_tx, first_rx) = mpsc::sync_channel::<Appearance>(1);

    // Its own thread and its own runtime: `compass ui` runs Iced on the main
    // thread before any runtime exists (ADR-0011), so there is nothing here to
    // spawn onto.
    let spawned = std::thread::Builder::new()
        .name("appearance".to_owned())
        .spawn(move || {
            let runtime = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(runtime) => runtime,
                Err(error) => {
                    tracing::warn!(%error, "no runtime for the appearance watcher");
                    return;
                }
            };
            runtime.block_on(pump(&first_tx, &sender));
        });

    if let Err(error) = spawned {
        tracing::warn!(%error, "could not start the appearance watcher");
        return (FALLBACK, None);
    }

    // `Err` is either the budget elapsing or the thread giving up, and both
    // mean the same thing here: draw the fallback, and take a later answer if
    // one comes.
    let first = first_rx.recv_timeout(FIRST_READ_BUDGET).unwrap_or(FALLBACK);
    (first, Some(link))
}

/// Read once, then follow, sending each answer on.
async fn pump(first: &mpsc::SyncSender<Appearance>, sender: &compass_ui::AppearanceSender) {
    let portals =
        match compass_portals::Portals::connect(compass_portals::PortalConfig::default()).await {
            Ok(portals) => portals,
            Err(error) => {
                tracing::debug!(%error, "no session bus; the launcher will not follow the desktop");
                return;
            }
        };
    portals.probe().await;
    let settings = match portals.settings() {
        Ok(settings) => settings,
        Err(error) => {
            tracing::info!(%error, "no Settings portal; the launcher will not follow the desktop");
            return;
        }
    };

    // Subscribed before the read, so a change between the two is not lost.
    let watch = match settings.watch().await {
        Ok(watch) => Some(watch),
        Err(error) => {
            tracing::warn!(%error, "could not watch the desktop appearance");
            None
        }
    };

    match settings.color_scheme().await {
        Ok(scheme) => {
            let appearance = to_appearance(scheme);
            // The receiver may already have given up waiting, in which case
            // the window drew the fallback and this correction is what fixes
            // it -- so both sends are attempted and neither is required.
            let _ = first.try_send(appearance);
            let _ = sender.send(appearance);
        }
        Err(error) => {
            tracing::info!(%error, "could not read the desktop appearance");
        }
    }

    let Some(watch) = watch else { return };
    let mut watch = Box::pin(watch);
    use futures_util::StreamExt;
    while let Some(scheme) = watch.next().await {
        if sender.send(to_appearance(scheme)).is_err() {
            // The window is gone. Nothing left to tell.
            return;
        }
    }
}

/// The portal's preference, as something to draw with.
///
/// Goes through `design::ColorScheme` rather than matching here, so the
/// mapping from the specification's numbers to a palette lives in one place
/// and is tested where the palettes are.
fn to_appearance(scheme: compass_portals::ColorScheme) -> Appearance {
    ColorScheme::from_portal(scheme.value()).appearance()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_modes_are_case_insensitive_and_unknown_values_follow_system() {
        assert_eq!(ColorMode::from_config("LIGHT"), ColorMode::Light);
        assert_eq!(ColorMode::from_config(" dark "), ColorMode::Dark);
        assert_eq!(ColorMode::from_config("solarized"), ColorMode::System);
        assert!(ColorMode::is_known("system"));
        assert!(!ColorMode::is_known("solarized"));
    }

    #[test]
    fn fixed_modes_do_not_create_a_portal_link() {
        let (light, light_link) = follow(ColorMode::Light);
        assert_eq!(light, Appearance::Light);
        assert!(light_link.is_none());

        let (dark, dark_link) = follow(ColorMode::Dark);
        assert_eq!(dark, Appearance::Dark);
        assert!(dark_link.is_none());
    }

    #[test]
    fn the_portals_preference_becomes_the_matching_palette() {
        assert_eq!(
            to_appearance(compass_portals::ColorScheme::PreferDark),
            Appearance::Dark
        );
        assert_eq!(
            to_appearance(compass_portals::ColorScheme::PreferLight),
            Appearance::Light
        );
    }

    #[test]
    fn no_preference_is_not_dark_by_accident() {
        // `design::ColorScheme` decides what "no preference" looks like, and
        // this is the test that fails if the two crates ever disagree about
        // which number means what.
        assert_eq!(
            to_appearance(compass_portals::ColorScheme::NoPreference),
            ColorScheme::NoPreference.appearance()
        );
    }
}
