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

/// How long the window waits for the first read before drawing anyway.
const FIRST_READ_BUDGET: Duration = Duration::from_millis(250);

/// The appearance to draw with when nothing has said otherwise.
///
/// Dark because that is what the launcher has always drawn. A machine with no
/// Settings portal keeps the appearance it had rather than changing the day
/// this landed.
const FALLBACK: Appearance = Appearance::Dark;

/// Start following the desktop's preference.
///
/// Returns what to draw the first frame with, and the link later changes
/// arrive on. The link is `None` when there is no way to follow the desktop at
/// all, which keeps "we are not following" visible in the flags rather than
/// hidden behind a channel nothing will ever send on.
pub fn follow() -> (Appearance, Option<AppearanceLink>) {
    let (link, sender) = AppearanceLink::new();
    let (first_tx, first_rx) = mpsc::sync_channel::<Appearance>(1);

    // Its own thread and its own runtime: `vicinae ui` runs Iced on the main
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
