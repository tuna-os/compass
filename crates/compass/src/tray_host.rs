//! Other applications' tray icons: the StatusNotifierItem host.
//!
//! Ports `SniTrayHost` (`src/server/src/services/tray-host/sni/`) onto the
//! `system-tray` crate, which does what the C++ does by hand: it serves a
//! `StatusNotifierWatcher` when nothing on the session bus owns the name (the
//! C++ `SniWatcher`), registers a host, follows items appearing and leaving
//! and their property and menu signals, and keeps each item's `dbusmenu`
//! layout. What is added around it:
//!
//! - an item's object path, which the crate's `Activate` assumes to be
//!   `/StatusNotifierItem`: it is read back from the watcher's
//!   `RegisteredStatusNotifierItems`, as the C++ `parseItemRef` keeps it;
//! - `IconThemePath`: a name the application ships beside itself is found
//!   in that directory (`TrayItem::resolveThemeIcons`);
//! - the largest pixmap, converted from the protocol's ARGB32 in network
//!   order to a PNG for the window (`toImage`).
//!
//! The host starts with the engine and on the engine's session bus. With no
//! bus it logs once and the requests are refused.
//!
//! Compass's own icon (`crate::tray_icon`) is left out of the list: it is
//! this process's, found by the connection's process id, and toggling the
//! launcher from the launcher's own list would only close it.

use std::path::Path;

use compass_core::tray_host::{self, Category, Status, TrayItem, TrayMenuItem};
use compass_ipc::{TrayItemInfo, TrayMenuEntry};
use system_tray::client::{ActivateRequest, Client};

/// The watcher's well-known name, path and interface.
const WATCHER: &str = "org.kde.StatusNotifierWatcher";
const WATCHER_PATH: &str = "/StatusNotifierWatcher";
const ITEM_IFACE: &str = "org.kde.StatusNotifierItem";

/// The engine's tray host, started once.
#[derive(Default)]
pub struct TrayHost {
    started: tokio::sync::OnceCell<Option<Started>>,
}

struct Started {
    client: Client,
    connection: zbus::Connection,
}

impl std::fmt::Debug for TrayHost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TrayHost")
            .field("started", &self.started.get().map(Option::is_some))
            .finish()
    }
}

/// Why a tray request cannot be answered.
pub const UNAVAILABLE: &str = "the tray is unavailable: the engine has no session bus";

impl TrayHost {
    /// Starts the host, if it has not been; `false` when it cannot.
    pub async fn start(&self) -> bool {
        self.started().await.is_some()
    }

    async fn started(&self) -> Option<&Started> {
        self.started
            .get_or_init(|| async {
                let started = async {
                    let client = Client::new().await.map_err(|e| e.to_string())?;
                    let connection = zbus::Connection::session()
                        .await
                        .map_err(|e| e.to_string())?;
                    Ok::<_, String>(Started { client, connection })
                }
                .await;
                started
                    .map_err(|reason| tracing::info!(%reason, "tray host not started"))
                    .ok()
            })
            .await
            .as_ref()
    }

    /// Every item but Compass's own, in key order, as the tray view lists
    /// them.
    ///
    /// # Errors
    ///
    /// [`UNAVAILABLE`] when the host could not start.
    pub async fn items(&self) -> Result<Vec<TrayItemInfo>, String> {
        let started = self.started().await.ok_or(UNAVAILABLE)?;
        let map = started.client.items();
        let mut items: Vec<(String, system_tray::item::StatusNotifierItem)> = map
            .lock()
            .map_err(|_| "the tray host's item list is poisoned".to_owned())?
            .iter()
            .map(|(key, (item, _))| (key.clone(), item.clone()))
            .collect();
        items.sort_by(|a, b| a.0.cmp(&b.0));
        let mut listed = Vec::with_capacity(items.len());
        for (key, item) in items {
            if !is_own(&started.connection, &key).await {
                listed.push(info(key, &item, &find_in_theme_path));
            }
        }
        Ok(listed)
    }

    /// Activates the item `key`, or its secondary activation.
    ///
    /// # Errors
    ///
    /// When the host is down, the item is gone or the call fails.
    pub async fn activate(&self, key: &str, secondary: bool) -> Result<(), String> {
        let started = self.started().await.ok_or(UNAVAILABLE)?;
        let path = item_path(&started.connection, key).await;
        let method = if secondary {
            "SecondaryActivate"
        } else {
            "Activate"
        };
        started
            .connection
            .call_method(
                Some(key),
                path.as_str(),
                Some(ITEM_IFACE),
                method,
                &(0i32, 0i32),
            )
            .await
            .map(|_| ())
            .map_err(|error| format!("{method} failed: {error}"))
    }

    /// The item's menu, flattened as the C++ tray view lists it.
    ///
    /// # Errors
    ///
    /// When the host is down or the item is gone.
    pub async fn menu(&self, key: &str) -> Result<Vec<TrayMenuEntry>, String> {
        let started = self.started().await.ok_or(UNAVAILABLE)?;
        let menu_path = menu_path(&started.client, key)?;
        let Some(menu_path) = menu_path else {
            return Ok(Vec::new());
        };
        // `AboutToShow` first, as the C++ does: an application may fill the
        // menu only when it is about to be seen.
        let _ = started
            .client
            .about_to_show_menuitem(key.to_owned(), menu_path, 0)
            .await;
        let map = started.client.items();
        let entries = map
            .lock()
            .map_err(|_| "the tray host's item list is poisoned".to_owned())?
            .get(key)
            .and_then(|(_, menu)| menu.as_ref())
            .map(|menu| menu.submenus.iter().map(menu_item).collect::<Vec<_>>())
            .unwrap_or_default();
        Ok(tray_host::flatten_menu(&entries)
            .into_iter()
            .map(|row| TrayMenuEntry {
                id: row.entry.id,
                toggled: (row.entry.toggle_type != tray_host::ToggleType::None)
                    .then_some(row.entry.toggle_state == 1),
                icon_name: Some(row.entry.icon_name).filter(|name| !name.is_empty()),
                label: row.label,
            })
            .collect())
    }

    /// Clicks entry `id` of the item's menu.
    ///
    /// # Errors
    ///
    /// When the host is down, the item has no menu or the call fails.
    pub async fn trigger(&self, key: &str, id: i32) -> Result<(), String> {
        let started = self.started().await.ok_or(UNAVAILABLE)?;
        let menu_path = menu_path(&started.client, key)?.ok_or("the tray item has no menu")?;
        started
            .client
            .activate(ActivateRequest::MenuItem {
                address: key.to_owned(),
                menu_path,
                submenu_id: id,
            })
            .await
            .map_err(|error| format!("the menu entry did not run: {error}"))
    }
}

/// Whether the item `key` is served by this process: Compass's own icon.
async fn is_own(connection: &zbus::Connection, key: &str) -> bool {
    let Ok(name) = zbus::names::BusName::try_from(key) else {
        return false;
    };
    let Ok(proxy) = zbus::fdo::DBusProxy::new(connection).await else {
        return false;
    };
    proxy
        .get_connection_unix_process_id(name)
        .await
        .is_ok_and(|pid| pid == std::process::id())
}

/// The item's menu path, `None` when it has none (`hasMenu`).
fn menu_path(client: &Client, key: &str) -> Result<Option<String>, String> {
    let map = client.items();
    let map = map
        .lock()
        .map_err(|_| "the tray host's item list is poisoned".to_owned())?;
    let (item, _) = map.get(key).ok_or("that tray item is gone")?;
    Ok(item
        .menu
        .clone()
        .filter(|path| !path.is_empty() && path != "/"))
}

/// The object path the item registered under, read back from the watcher;
/// `/StatusNotifierItem`, the specification's, when it gave only its name.
async fn item_path(connection: &zbus::Connection, key: &str) -> String {
    let registered: Vec<String> = async {
        let proxy = zbus::fdo::PropertiesProxy::builder(connection)
            .destination(WATCHER)
            .ok()?
            .path(WATCHER_PATH)
            .ok()?
            .build()
            .await
            .ok()?;
        let value = proxy
            .get(
                zbus::names::InterfaceName::try_from(WATCHER).ok()?,
                "RegisteredStatusNotifierItems",
            )
            .await
            .ok()?;
        Vec::<String>::try_from(value).ok()
    }
    .await
    .unwrap_or_default();
    path_for(key, &registered)
}

/// The item path from the watcher's list: `busname/path` entries split at
/// the first slash, as `parseItemRef`.
#[must_use]
pub fn path_for(key: &str, registered: &[String]) -> String {
    registered
        .iter()
        .filter_map(|entry| entry.split_once('/'))
        .find(|(name, _)| *name == key)
        .map_or_else(
            || "/StatusNotifierItem".to_owned(),
            |(_, path)| format!("/{path}"),
        )
}

/// One `StatusNotifierItem` as the tray view lists it.
pub fn info(
    key: String,
    item: &system_tray::item::StatusNotifierItem,
    find: &dyn Fn(&str, &str) -> String,
) -> TrayItemInfo {
    let mut model = TrayItem {
        bus_name: key.clone(),
        id: item.id.clone(),
        title: item.title.clone().unwrap_or_default(),
        status: match item.status {
            system_tray::item::Status::Passive => Status::Passive,
            system_tray::item::Status::NeedsAttention => Status::NeedsAttention,
            _ => Status::Active,
        },
        category: match item.category {
            system_tray::item::Category::Communications => Category::Communications,
            system_tray::item::Category::SystemServices => Category::SystemServices,
            system_tray::item::Category::Hardware => Category::Hardware,
            system_tray::item::Category::ApplicationStatus => Category::ApplicationStatus,
        },
        icon_name: item.icon_name.clone().unwrap_or_default(),
        icon_theme_path: item.icon_theme_path.clone().unwrap_or_default(),
        icon_pixmap: item.icon_pixmap.as_deref().and_then(pixmap_png),
        attention_icon_name: item.attention_icon_name.clone().unwrap_or_default(),
        attention_icon_pixmap: item.attention_icon_pixmap.as_deref().and_then(pixmap_png),
        tooltip_title: item
            .tool_tip
            .as_ref()
            .map(|tip| tip.title.clone())
            .unwrap_or_default(),
        tooltip_description: item
            .tool_tip
            .as_ref()
            .map(|tip| tip.description.clone())
            .unwrap_or_default(),
        item_is_menu: item.item_is_menu,
        menu_path: item.menu.clone().unwrap_or_default(),
        ..TrayItem::default()
    };
    model.resolve_theme_icons(find);
    let mut png = None;
    let icon = model.icon(|bytes| {
        png = Some(bytes.to_vec());
        String::new()
    });
    let (icon_path, icon_name, icon_png) = match icon.kind {
        compass_core::image_url::ImageUrlType::Local => (Some(icon.name), None, None),
        compass_core::image_url::ImageUrlType::DataUri => (None, None, png),
        _ => (None, Some(icon.name), None),
    };
    TrayItemInfo {
        title: tray_host::display_title(&model).to_owned(),
        subtitle: tray_host::display_subtitle(&model).to_owned(),
        attention: model.status == Status::NeedsAttention,
        has_menu: model.has_menu(),
        item_is_menu: model.item_is_menu,
        key,
        icon_path,
        icon_name,
        icon_png,
    }
}

/// The largest usable pixmap as a PNG (`toImage`): ARGB32 in network byte
/// order, one `u32` per pixel.
#[must_use]
pub fn pixmap_png(pixmaps: &[system_tray::item::IconPixmap]) -> Option<Vec<u8>> {
    let best = pixmaps
        .iter()
        .filter(|p| {
            p.width > 0
                && p.height > 0
                && p.pixels.len() >= usize::try_from(p.width * p.height * 4).unwrap_or(usize::MAX)
        })
        .max_by_key(|p| p.width)?;
    let width = u32::try_from(best.width).ok()?;
    let height = u32::try_from(best.height).ok()?;
    let rgba: Vec<u8> = best
        .pixels
        .chunks_exact(4)
        .take((width * height) as usize)
        .flat_map(|argb| [argb[1], argb[2], argb[3], argb[0]])
        .collect();
    let image = image::RgbaImage::from_raw(width, height, rgba)?;
    let mut png = Vec::new();
    image
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .ok()?;
    Some(png)
}

/// `findIconInThemePath`: `name` with an icon extension anywhere under
/// `theme_path`, the best by [`tray_host::best_icon`]; empty when none.
#[must_use]
pub fn find_in_theme_path(theme_path: &str, name: &str) -> String {
    let mut candidates = Vec::new();
    collect(Path::new(theme_path), name, &mut candidates, 0);
    tray_host::best_icon(&candidates)
        .map(|candidate| candidate.path.clone())
        .unwrap_or_default()
}

fn collect(dir: &Path, name: &str, out: &mut Vec<tray_host::IconCandidate>, depth: u8) {
    // Icon themes are `<size>/<context>/<name>`; deeper than that is not one.
    if depth > 4 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_dir() {
            collect(&path, name, out, depth + 1);
            continue;
        }
        if path.file_stem().and_then(|stem| stem.to_str()) != Some(name) {
            continue;
        }
        let extension = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
        out.push(tray_host::IconCandidate {
            path: path.to_string_lossy().into_owned(),
            extension,
            size,
        });
    }
}

/// `system-tray`'s menu entry as the C++'s `TrayMenuItem` (`toMenuItem`).
fn menu_item(entry: &system_tray::menu::MenuItem) -> TrayMenuItem {
    use system_tray::menu::{MenuType, ToggleState, ToggleType};
    TrayMenuItem {
        id: entry.id,
        label: entry.label.clone().unwrap_or_default(),
        enabled: entry.enabled,
        visible: entry.visible,
        separator: entry.menu_type == MenuType::Separator,
        submenu: entry.children_display.as_deref() == Some("submenu") || !entry.submenu.is_empty(),
        toggle_type: match entry.toggle_type {
            ToggleType::Checkmark => tray_host::ToggleType::Checkmark,
            ToggleType::Radio => tray_host::ToggleType::Radio,
            ToggleType::CannotBeToggled => tray_host::ToggleType::None,
        },
        toggle_state: match entry.toggle_state {
            ToggleState::On => 1,
            ToggleState::Off => 0,
            ToggleState::Indeterminate => -1,
        },
        icon_name: entry.icon_name.clone().unwrap_or_default(),
        icon_data: entry.icon_data.clone(),
        children: entry.submenu.iter().map(menu_item).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_items_path_is_read_back_from_the_watchers_list() {
        let registered = vec![
            ":1.7/org/ayatana/NotificationItem/app".to_owned(),
            "org.kde.StatusNotifierItem-9-1/StatusNotifierItem".to_owned(),
        ];
        assert_eq!(
            path_for(":1.7", &registered),
            "/org/ayatana/NotificationItem/app"
        );
        assert_eq!(path_for(":1.99", &registered), "/StatusNotifierItem");
    }

    #[test]
    fn the_largest_pixmap_becomes_a_png_with_its_channels_reordered() {
        use system_tray::item::IconPixmap;
        let small = IconPixmap {
            width: 1,
            height: 1,
            pixels: vec![255, 0, 0, 255],
        };
        // ARGB: opaque red, then half-transparent blue.
        let large = IconPixmap {
            width: 2,
            height: 1,
            pixels: vec![255, 255, 0, 0, 128, 0, 0, 255],
        };
        let short = IconPixmap {
            width: 9,
            height: 9,
            pixels: vec![0; 4],
        };
        let png = pixmap_png(&[small, large, short]).expect("a usable pixmap");
        let decoded = image::load_from_memory(&png).unwrap().to_rgba8();
        assert_eq!(decoded.dimensions(), (2, 1));
        assert_eq!(decoded.get_pixel(0, 0).0, [255, 0, 0, 255]);
        assert_eq!(decoded.get_pixel(1, 0).0, [0, 0, 255, 128]);
        assert!(pixmap_png(&[]).is_none());
    }

    #[test]
    fn an_icon_shipped_in_the_items_theme_path_is_found_there_svg_first() {
        let dir = tempfile::tempdir().unwrap();
        let sized = dir.path().join("hicolor/22x22/apps");
        std::fs::create_dir_all(&sized).unwrap();
        std::fs::write(sized.join("app-tray.png"), b"small").unwrap();
        std::fs::write(dir.path().join("app-tray.png"), b"bigger png").unwrap();
        let theme = dir.path().to_string_lossy().into_owned();
        assert_eq!(
            find_in_theme_path(&theme, "app-tray"),
            dir.path().join("app-tray.png").to_string_lossy()
        );
        std::fs::write(sized.join("app-tray.svg"), b"<svg/>").unwrap();
        assert!(find_in_theme_path(&theme, "app-tray").ends_with(".svg"));
        assert_eq!(find_in_theme_path(&theme, "other"), "");
    }
}
