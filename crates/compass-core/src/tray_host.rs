//! Other applications' tray icons: which picture to draw, and what their menu
//! labels really say.
//!
//! A port of `TrayItem` and `TrayMenuItem`
//! (`src/server/src/services/tray-host/`), minus the StatusNotifierItem D-Bus
//! plumbing and the image encoder.
//!
//! # Every rule here is a fallback, because every field is optional
//!
//! The StatusNotifierItem specification lets an application supply an icon by
//! name, by path, or as raw pixels, and lets it supply a second set for when
//! it wants attention — and applications supply whatever subset they feel
//! like. So the interesting logic is entirely about what to do when the
//! preferred thing is missing, and the last fallback has to be something that
//! always draws, or the tray shows a hole where an application is running.

use crate::image_url::{ImageUrl, ImageUrlType};

/// The icon shown when an application supplied nothing usable.
///
/// A generic executable, because a blank space in the tray reads as a bug in
/// *this* program rather than as a badly-behaved application.
pub const FALLBACK_ICON_NAME: &str = "application-x-executable";

/// The image extensions a theme directory is searched for.
pub const ICON_EXTENSIONS: &[&str] = &["svg", "png", "xpm"];

/// How much attention an item is asking for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Status {
    /// Present but idle; some hosts hide these.
    Passive,
    /// Ordinary.
    #[default]
    Active,
    /// Asking to be noticed.
    NeedsAttention,
}

/// What kind of application this is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Category {
    /// An ordinary application.
    #[default]
    ApplicationStatus,
    /// A chat or mail client.
    Communications,
    /// A system service.
    SystemServices,
    /// A hardware indicator.
    Hardware,
}

/// One tray icon belonging to another application.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TrayItem {
    /// The bus name it lives on.
    pub bus_name: String,
    /// Its object path.
    pub path: String,
    /// Its own id.
    pub id: String,
    /// Its title.
    pub title: String,
    /// How much attention it wants.
    pub status: Status,
    /// What kind of application it is.
    pub category: Category,
    /// The icon's name in the theme.
    pub icon_name: String,
    /// A directory to search for the icon, outside the theme.
    pub icon_theme_path: String,
    /// A resolved path to the icon.
    pub icon_path: String,
    /// Raw pixels, if it supplied them.
    pub icon_pixmap: Option<Vec<u8>>,
    /// The attention icon's name.
    pub attention_icon_name: String,
    /// A resolved path to the attention icon.
    pub attention_icon_path: String,
    /// Raw attention pixels.
    pub attention_icon_pixmap: Option<Vec<u8>>,
    /// The tooltip's heading.
    pub tooltip_title: String,
    /// The tooltip's body.
    pub tooltip_description: String,
    /// Whether the whole item is a menu.
    pub item_is_menu: bool,
    /// The menu's object path.
    pub menu_path: String,
}

impl TrayItem {
    /// What identifies this item.
    ///
    /// The bus name and the object path together, because one application can
    /// export several items on one connection.
    #[must_use]
    pub fn key(&self) -> String {
        format!("{}{}", self.bus_name, self.path)
    }

    /// Whether there is a menu to open.
    ///
    /// `/` does not count: it is what an application sends to mean "no menu"
    /// without leaving the field empty, and opening it gets an error.
    #[must_use]
    pub fn has_menu(&self) -> bool {
        !self.menu_path.is_empty() && self.menu_path != "/"
    }

    /// The icon to draw.
    ///
    /// # Each attention field falls back on its own
    ///
    /// An item needing attention uses its attention name, path and pixmap —
    /// but each only if that *particular* field is set, so an application
    /// supplying an attention name and no attention pixmap still gets its
    /// ordinary pixmap rather than nothing. Treating the attention set as all
    /// or nothing would blank icons that work today.
    #[must_use]
    pub fn icon(&self, encode_png: impl FnOnce(&[u8]) -> String) -> ImageUrl {
        let attention = self.status == Status::NeedsAttention;

        let name = if attention && !self.attention_icon_name.is_empty() {
            &self.attention_icon_name
        } else {
            &self.icon_name
        };
        let path = if attention && !self.attention_icon_path.is_empty() {
            &self.attention_icon_path
        } else {
            &self.icon_path
        };
        let pixmap = if attention && self.attention_icon_pixmap.is_some() {
            self.attention_icon_pixmap.as_ref()
        } else {
            self.icon_pixmap.as_ref()
        };

        // A resolved path first: it is the only source that is certainly the
        // icon this application meant, since a theme name can resolve to
        // something else entirely on another theme.
        if !path.is_empty() {
            return ImageUrl::local(path.clone());
        }
        if let Some(pixmap) = pixmap.filter(|bytes| !bytes.is_empty()) {
            return ImageUrl::new(ImageUrlType::DataUri, encode_png(pixmap));
        }
        if !name.is_empty() {
            return ImageUrl::new(ImageUrlType::System, name.clone());
        }
        ImageUrl::new(ImageUrlType::System, FALLBACK_ICON_NAME)
    }

    /// Resolve the icon names against `icon_theme_path`.
    ///
    /// An absolute name is already a path and is taken as one; anything else
    /// is looked for in the directory the application nominated.
    pub fn resolve_theme_icons(&mut self, find: impl Fn(&str, &str) -> String) {
        self.icon_path = resolve_icon(&self.icon_theme_path, &self.icon_name, &find);
        self.attention_icon_path =
            resolve_icon(&self.icon_theme_path, &self.attention_icon_name, &find);
    }
}

/// Resolve one icon name.
///
/// An empty name or an empty theme path resolves to nothing without searching,
/// as `findIconInThemePath`'s first line does — and it matters, because most
/// items have no attention icon and walking a theme directory for a name that
/// is not there is the most expensive nothing this service could do.
fn resolve_icon(theme_path: &str, name: &str, find: &impl Fn(&str, &str) -> String) -> String {
    if name.starts_with('/') {
        return name.to_owned();
    }
    if name.is_empty() || theme_path.is_empty() {
        return String::new();
    }
    find(theme_path, name)
}

/// One candidate file found while searching a theme directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IconCandidate {
    /// Its full path.
    pub path: String,
    /// Its extension, lowercased and without the dot.
    pub extension: String,
    /// Its size in bytes.
    pub size: u64,
}

/// Pick the best icon from what a theme directory search turned up.
///
/// # SVG wins immediately; otherwise the biggest file wins
///
/// An SVG is resolution-independent, so there is nothing to compare it
/// against and the search can stop at the first one. Among bitmaps the C++
/// uses *file size* as a proxy for resolution — crude, and right often enough,
/// since a 256px PNG is almost always a larger file than a 22px one.
#[must_use]
pub fn best_icon(candidates: &[IconCandidate]) -> Option<&IconCandidate> {
    let mut best: Option<&IconCandidate> = None;

    for candidate in candidates {
        if !ICON_EXTENSIONS.contains(&candidate.extension.as_str()) {
            continue;
        }
        if candidate.extension == "svg" {
            return Some(candidate);
        }
        // Strictly greater, so the first of two equal-sized files wins and the
        // choice does not depend on the order the directory happened to be
        // read in.
        if best.is_none_or(|current| candidate.size > current.size) {
            best = Some(candidate);
        }
    }

    best
}

/// How a menu item toggles, if it does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ToggleType {
    /// Not a toggle.
    #[default]
    None,
    /// A checkbox.
    Checkmark,
    /// One of a group.
    Radio,
}

/// One entry of another application's tray menu.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TrayMenuItem {
    /// Its id on the bus.
    pub id: i32,
    /// Its label, with mnemonics still in it.
    pub label: String,
    /// Whether it can be clicked.
    pub enabled: bool,
    /// Whether it is shown.
    pub visible: bool,
    /// Whether it is a dividing line.
    pub separator: bool,
    /// Whether it opens a submenu.
    pub submenu: bool,
    /// How it toggles.
    pub toggle_type: ToggleType,
    /// Its toggle state; -1 means indeterminate.
    pub toggle_state: i32,
    /// Its icon's name.
    pub icon_name: String,
    /// Its icon's raw bytes.
    pub icon_data: Option<Vec<u8>>,
    /// Its submenu's entries.
    pub children: Vec<TrayMenuItem>,
}

impl TrayMenuItem {
    /// An entry with the C++'s defaults: enabled, visible, not a toggle.
    #[must_use]
    pub fn new() -> Self {
        Self {
            enabled: true,
            visible: true,
            toggle_state: -1,
            ..Self::default()
        }
    }

    /// The label without its keyboard mnemonics.
    ///
    /// GTK and Qt mark the mnemonic with an underscore — `_File` — and escape
    /// a real one by doubling it. Showing the raw label would put stray
    /// underscores through the menu; stripping both would eat the real ones in
    /// a filename.
    #[must_use]
    pub fn plain_label(&self) -> String {
        let chars: Vec<char> = self.label.chars().collect();
        let mut out = String::with_capacity(self.label.len());
        let mut index = 0;

        while index < chars.len() {
            if chars[index] == '_' {
                if index + 1 < chars.len() && chars[index + 1] == '_' {
                    out.push('_');
                    index += 1;
                }
                index += 1;
                continue;
            }
            out.push(chars[index]);
            index += 1;
        }

        out
    }
}

/// One node of a `com.canonical.dbusmenu` layout, as the bus delivers it.
///
/// The properties are a loose map because that is what the protocol sends:
/// an application supplies only what it has an opinion about, and everything
/// absent takes its default. Which defaults those are is the substance of the
/// conversion below.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MenuLayoutNode {
    /// The node's id, used to send the click back.
    pub id: i32,
    /// Whatever properties the application set.
    pub properties: Vec<(String, MenuProperty)>,
    /// Its children, already fetched — `GetLayout` is asked for the whole
    /// tree at once with a depth of -1.
    pub children: Vec<MenuLayoutNode>,
}

/// A property value, in the three shapes this conversion reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MenuProperty {
    /// A string.
    Text(String),
    /// A boolean.
    Flag(bool),
    /// An integer.
    Number(i32),
    /// Raw bytes, for `icon-data`.
    Bytes(Vec<u8>),
}

impl MenuLayoutNode {
    fn get(&self, key: &str) -> Option<&MenuProperty> {
        self.properties
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value)
    }

    fn text(&self, key: &str) -> String {
        match self.get(key) {
            Some(MenuProperty::Text(value)) => value.clone(),
            _ => String::new(),
        }
    }

    fn flag(&self, key: &str, default: bool) -> bool {
        match self.get(key) {
            Some(MenuProperty::Flag(value)) => *value,
            _ => default,
        }
    }
}

/// The properties asked for when the layout is fetched.
///
/// Anything not on this list is not sent, so the list *is* the set of
/// properties the conversion can read — adding a case below without adding the
/// name here would read a property that never arrives.
pub const MENU_PROPERTIES: &[&str] = &[
    "label",
    "enabled",
    "visible",
    "type",
    "toggle-type",
    "toggle-state",
    "icon-name",
    "icon-data",
    "children-display",
];

/// Turn a layout node into a menu entry, recursively.
///
/// Every default here is the protocol's rather than a guess, and three of them
/// matter:
///
/// * `enabled` and `visible` default to **true**. An application that sends
///   neither wants an ordinary, clickable entry, and defaulting to false would
///   render a menu of grey nothing.
/// * `toggle-state` is read **only when the key is present**, because its
///   default is `-1` — indeterminate — and that is a different state from `0`,
///   which means off. A checkbox nobody has answered is not an unchecked one.
/// * `toggle-type` falls back to [`ToggleType::None`] for any value it does
///   not know, so a type added to the protocol later renders as a plain entry
///   instead of an empty checkbox.
#[must_use]
pub fn menu_item_from_layout(node: &MenuLayoutNode) -> TrayMenuItem {
    let mut item = TrayMenuItem::new();

    item.id = node.id;
    item.label = node.text("label");
    item.enabled = node.flag("enabled", true);
    item.visible = node.flag("visible", true);
    item.separator = node.text("type") == "separator";
    item.submenu = node.text("children-display") == "submenu";
    item.icon_name = node.text("icon-name");

    item.toggle_type = match node.text("toggle-type").as_str() {
        "checkmark" => ToggleType::Checkmark,
        "radio" => ToggleType::Radio,
        _ => ToggleType::None,
    };

    if let Some(MenuProperty::Number(state)) = node.get("toggle-state") {
        item.toggle_state = *state;
    }

    if let Some(MenuProperty::Bytes(data)) = node.get("icon-data")
        && !data.is_empty()
    {
        item.icon_data = Some(data.clone());
    }

    item.children = node.children.iter().map(menu_item_from_layout).collect();
    item
}

/// The entries of a fetched menu.
///
/// The root node is the menu itself and is never shown, so its *children* are
/// the menu — returning the root would put an unnamed entry above every menu.
/// A failed fetch is an empty menu rather than an error, because a tray icon
/// whose menu will not load should still be clickable.
#[must_use]
pub fn menu_from_layout(root: Option<&MenuLayoutNode>) -> Vec<TrayMenuItem> {
    root.map(|node| menu_item_from_layout(node).children)
        .unwrap_or_default()
}

/// What a tray item's row is titled: its title, else its id
/// (`trayItemTitle`).
#[must_use]
pub fn display_title(item: &TrayItem) -> &str {
    if item.title.is_empty() {
        &item.id
    } else {
        &item.title
    }
}

/// A tray item's second line: its tooltip's heading when that says something
/// the title does not, else the tooltip's body.
#[must_use]
pub fn display_subtitle(item: &TrayItem) -> &str {
    if !item.tooltip_title.is_empty() && item.tooltip_title != display_title(item) {
        &item.tooltip_title
    } else {
        &item.tooltip_description
    }
}

/// One clickable menu entry, labelled with the submenus it sits in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuRow {
    /// The entry.
    pub entry: TrayMenuItem,
    /// Its label without mnemonics, after its submenus' (`File › Open`).
    pub label: String,
}

/// The menu as the tray view lists it (`TrayMenuViewHost::flatten`): hidden
/// entries and separators left out, a submenu's entries in its place under
/// its label, and disabled or unlabelled entries dropped.
#[must_use]
pub fn flatten_menu(entries: &[TrayMenuItem]) -> Vec<MenuRow> {
    let mut rows = Vec::new();
    flatten_into(entries, "", &mut rows);
    rows
}

fn flatten_into(entries: &[TrayMenuItem], prefix: &str, out: &mut Vec<MenuRow>) {
    for entry in entries {
        if !entry.visible || entry.separator {
            continue;
        }
        let plain = entry.plain_label();
        let label = if prefix.is_empty() {
            plain
        } else {
            format!("{prefix} › {plain}")
        };
        if entry.submenu {
            flatten_into(&entry.children, &label, out);
            continue;
        }
        if !entry.enabled || label.is_empty() {
            continue;
        }
        out.push(MenuRow {
            entry: entry.clone(),
            label,
        });
    }
}
