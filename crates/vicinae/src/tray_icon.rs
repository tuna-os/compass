//! Compass's own tray icon: `TrayServiceLinux`.
//!
//! A StatusNotifierItem with the C++'s menu (`compass_core::tray`): toggle
//! the launcher, the version, About and Settings (the settings view, opened
//! as `vicinae://settings/open` opens it), the three community links, and
//! Quit, which is left out under systemd. Served by the `ksni` crate, which
//! does what the C++ does by hand: it exports the item and its `dbusmenu`,
//! owns an `org.kde.StatusNotifierItem-<pid>-<n>` name, registers with the
//! `StatusNotifierWatcher`, and registers again whenever a watcher appears.
//!
//! `tray.enabled` (the C++ `config::Tray`, on by default) is read when the
//! engine starts and applied at once when the settings view changes it
//! (`configChanged`'s `tray->show()` / `tray->hide()`): on, the item is
//! served; off, it leaves the bus.
//!
//! With no session bus the icon is logged once and never shown; the engine
//! carries on.

use std::sync::Arc;

use compass_core::tray::{self, Activation, EntryKind, MenuEntry};
use compass_ipc::{Request, Response, WindowCommand};
use ksni::TrayMethods;
use tokio::sync::{RwLock, mpsc, watch};

use crate::serve::{EngineState, forward};

/// The item's `Id`: the application's id, as the desktop file names it.
pub const ITEM_ID: &str = "com.vicinae.Vicinae";

/// The themed icon the item asks the host to draw: the one the package
/// installs (`hicolor/scalable/apps/com.vicinae.Vicinae.svg`).
pub const ICON_NAME: &str = "com.vicinae.Vicinae";

/// The pixmap sizes drawn for a host without that icon, the C++'s.
pub const PIXMAP_SIZES: [u32; 6] = [16, 22, 24, 32, 48, 64];

/// The application's icon, drawn into the pixmaps.
const ICON_SVG: &[u8] = include_bytes!("../../../extra/compass.svg");

/// Where the settings view opens, and its About tab.
const SETTINGS_LINK: &str = "vicinae://settings/open";

/// What the rest of the engine uses to show or hide the icon.
#[derive(Debug)]
pub struct Control {
    enabled: watch::Sender<Option<bool>>,
}

impl Default for Control {
    fn default() -> Self {
        Self {
            enabled: watch::Sender::new(None),
        }
    }
}

impl Control {
    /// `tray.enabled` is now `enabled`.
    pub fn set_enabled(&self, enabled: bool) {
        self.enabled.send_if_modified(|current| {
            let changed = *current != Some(enabled);
            *current = Some(enabled);
            changed
        });
    }
}

/// The deeplink an [`Activation::OpenSettings`] opens.
#[must_use]
pub fn settings_link(tab: Option<&str>) -> String {
    match tab {
        Some(tab) => format!("{SETTINGS_LINK}?tab={tab}"),
        None => SETTINGS_LINK.to_owned(),
    }
}

/// The item ksni serves: the menu model and where a click goes.
struct Item {
    entries: Vec<MenuEntry>,
    version: String,
    pixmaps: Vec<ksni::Icon>,
    clicks: mpsc::UnboundedSender<Activation>,
}

impl Item {
    fn click(&self, activation: Activation) {
        let _ = self.clicks.send(activation);
    }
}

impl ksni::Tray for Item {
    fn id(&self) -> String {
        ITEM_ID.to_owned()
    }

    fn title(&self) -> String {
        tray::APP_NAME.to_owned()
    }

    fn category(&self) -> ksni::Category {
        ksni::Category::ApplicationStatus
    }

    fn icon_name(&self) -> String {
        ICON_NAME.to_owned()
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        self.pixmaps.clone()
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip {
            icon_name: ICON_NAME.to_owned(),
            title: tray::APP_NAME.to_owned(),
            ..ksni::ToolTip::default()
        }
    }

    // `Activate` and `SecondaryActivate` both toggle, as the C++'s do.
    fn activate(&mut self, _x: i32, _y: i32) {
        self.click(Activation::Toggle);
    }

    fn secondary_activate(&mut self, _x: i32, _y: i32) {
        self.click(Activation::Toggle);
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        self.entries
            .iter()
            .map(|entry| {
                if entry.kind == EntryKind::Separator {
                    return ksni::MenuItem::Separator;
                }
                let activation = tray::activate(&self.entries, entry.id);
                ksni::menu::StandardItem {
                    label: tray::entry_label(entry.kind, &self.version),
                    enabled: tray::entry_enabled(entry.kind),
                    activate: Box::new(move |item: &mut Self| {
                        if let Some(activation) = activation.clone() {
                            item.click(activation);
                        }
                    }),
                    ..ksni::menu::StandardItem::default()
                }
                .into()
            })
            .collect()
    }

    fn watcher_offline(&self, reason: ksni::OfflineReason) -> bool {
        tracing::info!(
            ?reason,
            "no StatusNotifierWatcher yet; the tray icon appears when one does"
        );
        true
    }
}

/// The application's icon as the protocol's pixmaps: ARGB32 in network byte
/// order, one per size in [`PIXMAP_SIZES`].
#[must_use]
pub fn pixmaps() -> Vec<ksni::Icon> {
    use resvg::{tiny_skia, usvg};
    let Ok(tree) = usvg::Tree::from_data(ICON_SVG, &usvg::Options::default()) else {
        return Vec::new();
    };
    PIXMAP_SIZES
        .iter()
        .filter_map(|&size| {
            let mut pixmap = tiny_skia::Pixmap::new(size, size)?;
            #[allow(clippy::cast_precision_loss)]
            let side = size as f32;
            let tree_size = tree.size();
            let scale = (side / tree_size.width()).min(side / tree_size.height());
            resvg::render(
                &tree,
                tiny_skia::Transform::from_scale(scale, scale),
                &mut pixmap.as_mut(),
            );
            let data = pixmap
                .pixels()
                .iter()
                .flat_map(|pixel| {
                    let color = pixel.demultiply();
                    [color.alpha(), color.red(), color.green(), color.blue()]
                })
                .collect();
            let side = i32::try_from(size).ok()?;
            Some(ksni::Icon {
                width: side,
                height: side,
                data,
            })
        })
        .collect()
}

/// Reads `tray.enabled`, or its default when the file cannot be read.
async fn configured() -> bool {
    match tokio::task::spawn_blocking(compass_core::Config::load).await {
        Ok(Ok(config)) => config.tray().enabled(),
        _ => compass_core::config::DEFAULT_TRAY_ENABLED,
    }
}

/// Carries out a menu click or an activation.
pub async fn perform(
    state: &Arc<RwLock<EngineState>>,
    activation: Activation,
    quit: &mpsc::Sender<()>,
) -> Response {
    match activation {
        Activation::Toggle => {
            let slot = state.read().await.window_slot();
            forward(&slot, WindowCommand::Toggle, "toggle a window").await
        }
        Activation::OpenSettings { tab } => {
            let slot = state.read().await.window_slot();
            let link = settings_link(tab.as_deref());
            forward(&slot, WindowCommand::Deeplink(link), "open the settings").await
        }
        Activation::OpenLink(link) => {
            crate::serve::handle(
                state,
                Request::OpenUrl {
                    url: link.url().to_owned(),
                },
            )
            .await
        }
        Activation::Quit => {
            tracing::info!("quit from the tray");
            let _ = quit.try_send(());
            Response::Ack
        }
    }
}

/// Serves the icon for as long as the engine runs, as `tray.enabled` says,
/// and carries out what is clicked. `quit` stops the engine.
pub async fn run(state: Arc<RwLock<EngineState>>, quit: mpsc::Sender<()>) {
    let control = state.read().await.tray_icon();
    control.set_enabled(configured().await);
    let mut enabled = control.enabled.subscribe();
    let (clicks_tx, mut clicks) = mpsc::unbounded_channel();
    // One at a time and in order: a toggle and then a settings click reach
    // the window as they were made.
    {
        let state = Arc::clone(&state);
        tokio::spawn(async move {
            while let Some(activation) = clicks.recv().await {
                if let Response::Error(error) = perform(&state, activation, &quit).await {
                    tracing::warn!(error = %error.message, "a tray menu entry did nothing");
                }
            }
        });
    }
    let under_systemd = std::env::var_os(tray::SYSTEMD_INVOCATION_ENV).is_some();
    let mut handle: Option<ksni::Handle<Item>> = None;
    loop {
        {
            let on = enabled.borrow_and_update().unwrap_or(false);
            match (on, handle.take()) {
                (true, None) => {
                    let item = Item {
                        entries: tray::menu_entries(under_systemd),
                        version: env!("CARGO_PKG_VERSION").to_owned(),
                        pixmaps: pixmaps(),
                        clicks: clicks_tx.clone(),
                    };
                    // Inside a Flatpak the sandbox may not own the
                    // specification's name; the item then registers under its
                    // unique name, as ksni recommends.
                    let spawned = item
                        .disable_dbus_name(crate::input_server::in_flatpak())
                        .assume_sni_available(true)
                        .spawn()
                        .await;
                    match spawned {
                        Ok(served) => {
                            tracing::info!("tray icon shown");
                            handle = Some(served);
                        }
                        Err(error) => {
                            tracing::info!(%error, "the tray icon cannot be shown");
                        }
                    }
                }
                (false, Some(served)) => {
                    served.shutdown().await;
                    tracing::info!("tray icon hidden");
                }
                (_, kept) => handle = kept,
            }
        }
        if enabled.changed().await.is_err() {
            break;
        }
    }
    if let Some(served) = handle {
        served.shutdown().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_settings_entries_open_the_settings_view_by_its_deeplink() {
        assert_eq!(settings_link(None), "vicinae://settings/open");
        assert_eq!(
            settings_link(Some("about")),
            "vicinae://settings/open?tab=about"
        );
        for tab in [None, Some("about")] {
            let link = settings_link(tab);
            assert_eq!(
                compass_core::settings_catalog::parse_settings_link(&link),
                Some(tab.map(str::to_owned)),
                "the view reads back what the tray opens: {link}"
            );
        }
    }

    #[test]
    fn the_pixmaps_are_the_cpp_sizes_in_argb_network_order() {
        let pixmaps = pixmaps();
        let sizes: Vec<i32> = pixmaps.iter().map(|icon| icon.width).collect();
        assert_eq!(sizes, [16, 22, 24, 32, 48, 64]);
        for icon in &pixmaps {
            assert_eq!(icon.width, icon.height);
            let side = usize::try_from(icon.width).unwrap();
            assert_eq!(icon.data.len(), side * side * 4);
            // The corner is outside the round icon: transparent.
            assert_eq!(icon.data[0], 0, "alpha comes first");
            // The centre is inside the round icon: opaque.
            let centre = (side / 2 * side + side / 2) * 4;
            assert_eq!(icon.data[centre], 255, "an opaque centre");
        }
        // At the largest size, the needle's red half above the centre
        // (#FF5D73): red is the byte after alpha, blue the last.
        let largest = pixmaps.last().unwrap();
        let side = usize::try_from(largest.width).unwrap();
        let needle = ((side * 88 / 256) * side + side / 2) * 4;
        let [alpha, red, green, blue] = largest.data[needle..needle + 4] else {
            unreachable!()
        };
        assert_eq!(alpha, 255);
        assert!(
            red > 200 && red > green && red > blue,
            "{:?}",
            [alpha, red, green, blue]
        );
    }

    #[test]
    fn the_menu_is_the_models_with_its_labels_and_actions() {
        let (clicks, mut seen) = mpsc::unbounded_channel();
        let mut item = Item {
            entries: tray::menu_entries(false),
            version: "1.2.3".to_owned(),
            pixmaps: Vec::new(),
            clicks,
        };
        let menu = ksni::Tray::menu(&item);
        let labels: Vec<Option<String>> = menu
            .iter()
            .map(|entry| match entry {
                ksni::MenuItem::Standard(item) => Some(item.label.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(
            labels,
            [
                Some("Toggle Compass".to_owned()),
                Some("Compass 1.2.3".to_owned()),
                None,
                Some("About Compass".to_owned()),
                Some("Settings…".to_owned()),
                None,
                Some("Sponsor Vicinae".to_owned()),
                Some("Join the Discord".to_owned()),
                Some("Follow on X".to_owned()),
                None,
                Some("Quit Compass".to_owned()),
            ]
        );
        let ksni::MenuItem::Standard(version) = &menu[1] else {
            panic!("the version is an entry");
        };
        assert!(!version.enabled, "the version is there to be read");
        for index in [0, 4, 3, 10] {
            let ksni::MenuItem::Standard(entry) = &menu[index] else {
                panic!("{index} is an entry");
            };
            (entry.activate)(&mut item);
        }
        ksni::Tray::activate(&mut item, 0, 0);
        let mut got = Vec::new();
        while let Ok(activation) = seen.try_recv() {
            got.push(activation);
        }
        assert_eq!(
            got,
            [
                Activation::Toggle,
                Activation::OpenSettings { tab: None },
                Activation::OpenSettings {
                    tab: Some("about".to_owned())
                },
                Activation::Quit,
                Activation::Toggle,
            ]
        );
    }

    #[test]
    fn a_supervised_engine_offers_no_quit() {
        let (clicks, _seen) = mpsc::unbounded_channel();
        let item = Item {
            entries: tray::menu_entries(true),
            version: String::new(),
            pixmaps: Vec::new(),
            clicks,
        };
        assert!(ksni::Tray::menu(&item).iter().all(|entry| match entry {
            ksni::MenuItem::Standard(item) => item.label != tray::QUIT_LABEL,
            _ => true,
        }));
    }

    #[tokio::test]
    async fn turning_the_setting_on_and_off_is_seen_once_each() {
        let control = Control::default();
        let mut seen = control.enabled.subscribe();
        control.set_enabled(true);
        assert!(seen.has_changed().unwrap());
        assert_eq!(*seen.borrow_and_update(), Some(true));
        control.set_enabled(true);
        assert!(!seen.has_changed().unwrap(), "the same value is no change");
        control.set_enabled(false);
        assert_eq!(*seen.borrow_and_update(), Some(false));
    }
}
