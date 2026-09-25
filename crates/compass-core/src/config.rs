//! The `vicinae.json` user configuration.
//!
//! The schema is the one in `docs/rust-engine/PLAN.md` §5.2:
//!
//! ```json
//! {
//!   "launcher": {
//!     "hotkey": "super+space",
//!     "close_on_focus_loss": false,
//!     "max_results": 50,
//!     "keybinding": "default",
//!     "wrap_navigation": false,
//!     "quick_launch": true,
//!     "appearance": { "color_scheme": "system", "preset": "gnome", "icons": false }
//!   },
//!   "extensions": {
//!     "auto_update": true,
//!     "installed": ["com.example.clock"]
//!   }
//! }
//! ```
//!
//! The JSON Schema for this file is generated from these types ([`json_schema`]) and published at
//! `packaging/schema/vicinae.schema.json`, which [`SCHEMA_URL`] points at. The C++ engine's
//! `settings.json` is migrated into this shape by [`crate::config_migration`].
//!
//! Two properties drive the design:
//!
//! * **Every field is optional and has a documented default** ([`DEFAULT_HOTKEY`],
//!   [`DEFAULT_CLOSE_ON_FOCUS_LOSS`], [`DEFAULT_MAX_RESULTS`], [`DEFAULT_AUTO_UPDATE`]). An empty
//!   file, an empty object, or a missing file all yield a working [`Config`].
//! * **Unknown fields survive a round trip.** A config written by a newer build must not be
//!   silently truncated when an older build reads and rewrites it, so every struct captures the
//!   keys it does not know in a `serde(flatten)` map and writes them back out. This is why the
//!   getters take the "absent means default" decision at read time rather than at parse time:
//!   materialising defaults into the file would rewrite a user's config with values they never
//!   chose.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Default global toggle hotkey. Matches the Phase 1 gate in the plan (Super+Space on GNOME).
pub const DEFAULT_HOTKEY: &str = "super+space";

/// Default for `launcher.close_on_focus_loss`; mirrors the C++ `Config::closeOnFocusLoss`.
pub const DEFAULT_CLOSE_ON_FOCUS_LOSS: bool = false;

/// Default for `launcher.max_results`.
pub const DEFAULT_MAX_RESULTS: usize = 50;

/// Default for `launcher.wrap_navigation`.
///
/// `Config::wrapNavigation` is `false` in the C++: the selection clamps at the
/// first and last row rather than going round. See [`crate::list_navigation`].
pub const DEFAULT_WRAP_NAVIGATION: bool = false;

/// Default for `launcher.quick_launch`.
///
/// On, per #87: Ctrl+1..9 launches the first through ninth result without
/// arrowing to it. It costs nothing when unused -- the chords are otherwise
/// unbound -- and is a real speed-up once learned.
///
/// This has no C++ counterpart to match. It sits directly under `launcher`
/// rather than under an `appearance` section because it is behaviour, not
/// appearance: it changes what a keystroke does, not what a row looks like.
pub const DEFAULT_QUICK_LAUNCH: bool = true;

/// Default for `launcher.appearance.preset`.
///
/// The GNOME preset, per #83: Spotlight-simple, and recognisably the desktop
/// it ships on. It resolves to exactly the geometry that ships today, so the
/// default appearance is unchanged by the presets existing.
pub const DEFAULT_PRESET: &str = "gnome";

/// Default for `launcher.appearance.color_scheme`.
///
/// System follows the desktop's native light/dark preference. The string is
/// deliberately kept in `compass-core` rather than parsed here: the UI owns
/// the palette, while the config crate owns only the durable schema.
pub const DEFAULT_COLOR_SCHEME: &str = "system";

/// Default for `launcher.appearance.theme`.
///
/// `system` follows the OS native appearance; other values name a curated
/// palette from the #153 assortment (catppuccin, dracula, nord, gruvbox,
/// tokyo-night, solarized).
pub const DEFAULT_THEME: &str = "system";

/// Default for `launcher.appearance.icons`.
///
/// On for recognizable application results. The row already reserves the space
/// -- an unresolved or disabled
/// icon draws the application's initial in a tinted square of the same size --
/// so turning this on changes what is in the slot, not the launcher's
/// footprint.
pub const DEFAULT_ICONS: bool = true;

/// Whether the launcher background is translucent when nothing says otherwise.
///
/// Off, matching the Spotlight-simple default. **Translucency, not blur:** see
/// [`AppearanceConfig::tint`] for why the key is not called `blur`.
pub const DEFAULT_TINT: bool = false;

/// Default for `launcher.keybinding`.
///
/// The literal the C++ writes, and the one `KeyBindingService::getMode` reads
/// as "the platform default" -- which on Linux is the vim chords. See
/// [`crate::keybinding`].
pub const DEFAULT_KEYBINDING: &str = "default";

/// Default for `extensions.auto_update`.
pub const DEFAULT_AUTO_UPDATE: bool = true;

/// The favourites when the file sets none: the C++ default file's
/// `favorites`, Clipboard History by its C++ id.
pub const DEFAULT_FAVORITES: &[&str] = &["clipboard:history"];

/// Default for `launcher.clock.enabled`: the C++ default file shows it.
pub const DEFAULT_CLOCK_ENABLED: bool = true;

/// Default for `launcher.clock.interval`, in seconds.
pub const DEFAULT_CLOCK_INTERVAL: u64 = 60;

/// The format the clock uses when none is set: the C++ default file's
/// "localized hh:mm", as a Qt format.
pub const DEFAULT_CLOCK_FORMAT: &str = "hh:mm";

/// Path of the config file relative to `$XDG_CONFIG_HOME`.
pub const CONFIG_RELATIVE_PATH: &str = "vicinae/vicinae.json";

/// Where the published JSON Schema for `vicinae.json` lives, as a `$schema` value.
///
/// The file behind it is `packaging/schema/vicinae.schema.json`, generated from these types by
/// [`json_schema`] and held to them by the `config_schema` test.
pub const SCHEMA_URL: &str =
    "https://raw.githubusercontent.com/tuna-os/compass/main/packaging/schema/vicinae.schema.json";

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
/// Per-entrypoint root search settings, keyed by entrypoint id under its provider.
struct RootEntrypointSettings {
    /// Whether the entrypoint appears in root search.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    enabled: Option<bool>,
    /// An extra search term the entrypoint answers to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    alias: Option<String>,
    /// A hotkey that runs the entrypoint directly, e.g. `ctrl+shift+c`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    shortcut: Option<String>,
    #[serde(flatten)]
    unknown: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
/// Per-provider root search settings, keyed by provider id (e.g. `applications`).
struct RootProviderSettings {
    /// Whether the provider contributes to root search at all.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    enabled: Option<bool>,
    /// Settings for individual entrypoints of this provider.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    entrypoints: Option<BTreeMap<String, RootEntrypointSettings>>,
    #[serde(flatten)]
    unknown: BTreeMap<String, Value>,
}

/// Everything that can go wrong loading or saving a [`Config`].
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    /// The file exists but could not be read.
    #[error("could not read the configuration at {path}")]
    Read {
        /// The offending path.
        path: PathBuf,
        /// The underlying I/O error.
        #[source]
        source: std::io::Error,
    },

    /// The file could not be written.
    #[error("could not write the configuration to {path}")]
    Write {
        /// The offending path.
        path: PathBuf,
        /// The underlying I/O error.
        #[source]
        source: std::io::Error,
    },

    /// The file is not valid JSON, or is JSON of the wrong shape.
    ///
    /// The message names the problem and where it is, e.g.
    /// `invalid configuration at /home/u/.config/vicinae/vicinae.json: line 3 column 18: expected
    /// value`.
    #[error("invalid configuration at {path}: line {line} column {column}: {message}")]
    Parse {
        /// The offending path.
        path: PathBuf,
        /// 1-based line of the problem, `0` when the parser could not place it.
        line: usize,
        /// 1-based column of the problem, `0` when the parser could not place it.
        column: usize,
        /// What the JSON parser objected to.
        message: String,
        /// The underlying serde error.
        #[source]
        source: serde_json::Error,
    },

    /// `$XDG_CONFIG_HOME` (and its `$HOME` fallback) could not be resolved.
    #[error("could not determine the user configuration directory")]
    NoConfigDir,
}

/// The `launcher` section.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct LauncherConfig {
    /// The global chord that toggles the launcher, as `modifier+key`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("default" = DEFAULT_HOTKEY, "examples" = ["super+space", "alt+space"]))]
    hotkey: Option<String>,
    /// Whether the launcher hides when it loses keyboard focus.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("default" = DEFAULT_CLOSE_ON_FOCUS_LOSS))]
    close_on_focus_loss: Option<bool>,
    /// How many results the launcher shows. `0` shows none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("default" = DEFAULT_MAX_RESULTS))]
    max_results: Option<usize>,
    /// The navigation chord scheme: `default` (the platform's, vim on Linux), `vim` or `emacs`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("default" = DEFAULT_KEYBINDING, "examples" = ["default", "vim", "emacs"]))]
    keybinding: Option<String>,
    /// Whether moving past the last result selects the first, and the reverse.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("default" = DEFAULT_WRAP_NAVIGATION))]
    wrap_navigation: Option<bool>,
    /// Whether Ctrl+1..9 launches the first through ninth result.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("default" = DEFAULT_QUICK_LAUNCH))]
    quick_launch: Option<bool>,
    /// Colour mode and row presentation.
    #[serde(default, skip_serializing_if = "AppearanceConfig::is_empty")]
    appearance: AppearanceConfig,
    /// The clock the root search shows in its status bar.
    #[serde(default, skip_serializing_if = "ClockConfig::is_empty")]
    clock: ClockConfig,

    /// Keys this build does not know about, preserved verbatim.
    #[serde(flatten)]
    unknown: BTreeMap<String, Value>,
}

/// The `launcher.clock` section (the C++ `launcher_window.clock`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ClockConfig {
    /// Whether the clock is shown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("default" = DEFAULT_CLOCK_ENABLED))]
    enabled: Option<bool>,
    /// A Qt date-time format, e.g. `hh:mm:ss`; `hh:mm` when unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("examples" = ["hh:mm:ss", "ddd hh:mm"]))]
    format: Option<String>,
    /// How often the clock is redrawn, in seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("default" = DEFAULT_CLOCK_INTERVAL))]
    interval: Option<u64>,
}

impl ClockConfig {
    /// Whether the clock is shown. Defaults to [`DEFAULT_CLOCK_ENABLED`].
    #[must_use]
    pub fn enabled(&self) -> bool {
        self.enabled.unwrap_or(DEFAULT_CLOCK_ENABLED)
    }

    /// The Qt format it is drawn in. Defaults to [`DEFAULT_CLOCK_FORMAT`].
    #[must_use]
    pub fn format(&self) -> &str {
        self.format.as_deref().unwrap_or(DEFAULT_CLOCK_FORMAT)
    }

    /// Seconds between redraws, never zero. Defaults to
    /// [`DEFAULT_CLOCK_INTERVAL`].
    #[must_use]
    pub fn interval(&self) -> u64 {
        self.interval.unwrap_or(DEFAULT_CLOCK_INTERVAL).max(1)
    }

    fn is_empty(&self) -> bool {
        self.enabled.is_none() && self.format.is_none() && self.interval.is_none()
    }
}

/// The `launcher.appearance` section: colour mode and row presentation, not behavior.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AppearanceConfig {
    /// Light or dark: `system` follows the desktop, `light` and `dark` force one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("default" = DEFAULT_COLOR_SCHEME, "examples" = ["system", "light", "dark"]))]
    color_scheme: Option<String>,
    /// The palette: `system` follows the desktop, anything else names a curated theme.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend(
        "default" = DEFAULT_THEME,
        "examples" = ["system", "catppuccin", "dracula", "nord", "gruvbox", "tokyo-night", "solarized"]
    ))]
    theme: Option<String>,
    /// The named layout preset supplying the defaults for this section.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("default" = DEFAULT_PRESET, "examples" = ["gnome", "raycast", "flow", "rofi"]))]
    preset: Option<String>,
    /// Whether result rows show the application's icon. The preset supplies the default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("default" = DEFAULT_ICONS))]
    icons: Option<bool>,
    /// Whether the launcher background is translucent. Translucency, not blur.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("default" = DEFAULT_TINT))]
    tint: Option<bool>,

    /// Keys this build does not know about, preserved verbatim.
    #[serde(flatten)]
    unknown: BTreeMap<String, Value>,
}

impl AppearanceConfig {
    /// The configured colour scheme, or [`DEFAULT_COLOR_SCHEME`].
    #[must_use]
    pub fn color_scheme(&self) -> &str {
        self.color_scheme.as_deref().unwrap_or(DEFAULT_COLOR_SCHEME)
    }

    /// The explicit `launcher.appearance.color_scheme`, if one was written.
    #[must_use]
    pub fn color_scheme_override(&self) -> Option<&str> {
        self.color_scheme.as_deref()
    }

    /// Sets `launcher.appearance.color_scheme`. `None` restores the System default.
    pub fn set_color_scheme(&mut self, value: Option<String>) -> &mut Self {
        self.color_scheme = value;
        self
    }

    /// The configured theme, or [`DEFAULT_THEME`].
    ///
    /// `system` follows the desktop; any other value names a curated palette
    /// from the #153 assortment. Kept as a string here so `compass-ui` owns
    /// the palette table.
    #[must_use]
    pub fn theme(&self) -> &str {
        self.theme.as_deref().unwrap_or(DEFAULT_THEME)
    }

    /// The explicit `launcher.appearance.theme`, if one was written.
    #[must_use]
    pub fn theme_override(&self) -> Option<&str> {
        self.theme.as_deref()
    }

    /// Sets `launcher.appearance.theme`. `None` restores System.
    pub fn set_theme(&mut self, value: Option<String>) -> &mut Self {
        self.theme = value;
        self
    }

    /// The named preset supplying the defaults for this section (#84).
    ///
    /// Defaults to [`DEFAULT_PRESET`]. Returned as written rather than parsed
    /// here: `compass-core` has no opinion about what a preset looks like, and
    /// `compass_ui::preset` is where the names are known.
    #[must_use]
    pub fn preset(&self) -> &str {
        self.preset.as_deref().unwrap_or(DEFAULT_PRESET)
    }

    /// Sets `launcher.appearance.preset`. `None` removes the key.
    pub fn set_preset(&mut self, value: Option<String>) -> &mut Self {
        self.preset = value;
        self
    }

    /// The explicit `launcher.appearance.icons`, if one was written.
    ///
    /// Distinct from [`AppearanceConfig::icons`]: a preset supplies the
    /// default, so the resolver needs to know whether the user wrote the key
    /// at all rather than what it falls back to.
    #[must_use]
    pub fn icons_override(&self) -> Option<bool> {
        self.icons
    }

    /// Whether result rows show the application's icon (#85).
    ///
    /// Defaults to [`DEFAULT_ICONS`].
    #[must_use]
    pub fn icons(&self) -> bool {
        self.icons.unwrap_or(DEFAULT_ICONS)
    }

    /// Sets `launcher.appearance.icons`. `None` removes the key.
    pub fn set_icons(&mut self, value: Option<bool>) -> &mut Self {
        self.icons = value;
        self
    }

    /// The explicit `launcher.appearance.tint`, if one was written.
    #[must_use]
    pub fn tint_override(&self) -> Option<bool> {
        self.tint
    }

    /// Whether the launcher background is translucent (#86).
    ///
    /// **This is translucency, and the key is deliberately not called `blur`.**
    /// Mutter has no blur protocol — what GNOME extensions call blur is the
    /// shell compositing its own surfaces, which a Wayland client cannot
    /// request. The only real alternative was capturing the screen through a
    /// portal and blurring it ourselves, which costs a permission a launcher
    /// should never need. A `blur` key that does not blur would produce
    /// correct bug reports forever.
    ///
    /// Defaults to [`DEFAULT_TINT`].
    #[must_use]
    pub fn tint(&self) -> bool {
        self.tint.unwrap_or(DEFAULT_TINT)
    }

    /// Sets `launcher.appearance.tint`. `None` removes the key.
    pub fn set_tint(&mut self, value: Option<bool>) -> &mut Self {
        self.tint = value;
        self
    }

    /// Keys present in the file that this build does not understand.
    #[must_use]
    pub fn unknown_fields(&self) -> &BTreeMap<String, Value> {
        &self.unknown
    }

    /// Whether the section carries nothing at all, known or unknown.
    ///
    /// Destructured for the reason [`LauncherConfig::is_empty`] gives.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        let Self {
            color_scheme,
            theme,
            preset,
            icons,
            tint,
            unknown,
        } = self;
        color_scheme.is_none()
            && theme.is_none()
            && preset.is_none()
            && icons.is_none()
            && tint.is_none()
            && unknown.is_empty()
    }
}

impl LauncherConfig {
    /// The configured hotkey, or [`DEFAULT_HOTKEY`].
    #[must_use]
    pub fn hotkey(&self) -> &str {
        self.hotkey.as_deref().unwrap_or(DEFAULT_HOTKEY)
    }

    /// Whether the launcher closes when it loses focus. Defaults to
    /// [`DEFAULT_CLOSE_ON_FOCUS_LOSS`].
    #[must_use]
    pub fn close_on_focus_loss(&self) -> bool {
        self.close_on_focus_loss
            .unwrap_or(DEFAULT_CLOSE_ON_FOCUS_LOSS)
    }

    /// The navigation chord scheme, as [`crate::keybinding::Scheme::from_config`] reads it.
    ///
    /// Defaults to [`DEFAULT_KEYBINDING`], which is the platform default and
    /// therefore the vim chords on Linux.
    #[must_use]
    pub fn keybinding(&self) -> &str {
        self.keybinding.as_deref().unwrap_or(DEFAULT_KEYBINDING)
    }

    /// Whether the selection wraps at the ends of a list.
    ///
    /// Defaults to [`DEFAULT_WRAP_NAVIGATION`], which is the C++'s default:
    /// clamp.
    #[must_use]
    pub fn wrap_navigation(&self) -> bool {
        self.wrap_navigation.unwrap_or(DEFAULT_WRAP_NAVIGATION)
    }

    /// Whether Ctrl+1..9 launches the first through ninth result (#87).
    ///
    /// Defaults to [`DEFAULT_QUICK_LAUNCH`].
    #[must_use]
    pub fn quick_launch(&self) -> bool {
        self.quick_launch.unwrap_or(DEFAULT_QUICK_LAUNCH)
    }

    /// The scheme [`keybinding`](Self::keybinding) names.
    #[must_use]
    pub fn keybinding_scheme(&self) -> crate::keybinding::Scheme {
        crate::keybinding::Scheme::from_config(self.keybinding())
    }

    /// How many results the launcher shows. Defaults to [`DEFAULT_MAX_RESULTS`].
    ///
    /// A configured `0` is honoured as "show nothing"; only an absent key falls back.
    #[must_use]
    pub fn max_results(&self) -> usize {
        self.max_results.unwrap_or(DEFAULT_MAX_RESULTS)
    }

    /// Sets `launcher.hotkey`. `None` removes the key, restoring the default.
    pub fn set_hotkey(&mut self, hotkey: Option<String>) -> &mut Self {
        self.hotkey = hotkey;
        self
    }

    /// Sets `launcher.close_on_focus_loss`. `None` removes the key.
    pub fn set_close_on_focus_loss(&mut self, value: Option<bool>) -> &mut Self {
        self.close_on_focus_loss = value;
        self
    }

    /// Sets `launcher.max_results`. `None` removes the key.
    pub fn set_max_results(&mut self, value: Option<usize>) -> &mut Self {
        self.max_results = value;
        self
    }

    /// The `launcher.appearance` section.
    #[must_use]
    pub fn appearance(&self) -> &AppearanceConfig {
        &self.appearance
    }

    /// The `launcher.appearance` section, mutably.
    pub fn appearance_mut(&mut self) -> &mut AppearanceConfig {
        &mut self.appearance
    }

    /// Keys present in the file that this build does not understand.
    #[must_use]
    pub fn unknown_fields(&self) -> &BTreeMap<String, Value> {
        &self.unknown
    }

    /// Whether the section carries nothing at all, known or unknown.
    ///
    /// Destructured rather than written as a chain of `self.field.is_none()`,
    /// and that is the point rather than a style choice. This is
    /// `Config`'s `skip_serializing_if` for the whole section, so a field it
    /// forgets is a field a read-modify-write **deletes from the user's file**.
    /// It had forgotten three: a config holding only `keybinding`,
    /// `wrap_navigation` or `quick_launch` serialised back out as `{}`. A
    /// destructuring binding makes the next added field a compile error
    /// instead of silent data loss.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        let Self {
            hotkey,
            close_on_focus_loss,
            max_results,
            keybinding,
            wrap_navigation,
            quick_launch,
            appearance,
            clock,
            unknown,
        } = self;
        hotkey.is_none()
            && close_on_focus_loss.is_none()
            && max_results.is_none()
            && keybinding.is_none()
            && wrap_navigation.is_none()
            && quick_launch.is_none()
            && appearance.is_empty()
            && clock.is_empty()
            && unknown.is_empty()
    }

    /// The `launcher.clock` section.
    #[must_use]
    pub fn clock(&self) -> &ClockConfig {
        &self.clock
    }
}

/// The `extensions` section.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ExtensionsConfig {
    /// Whether installed extensions update themselves.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("default" = DEFAULT_AUTO_UPDATE))]
    auto_update: Option<bool>,
    /// The ids of the installed extensions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    installed: Option<Vec<String>>,

    /// Keys this build does not know about, preserved verbatim.
    #[serde(flatten)]
    unknown: BTreeMap<String, Value>,
}

impl ExtensionsConfig {
    /// Whether installed extensions update themselves. Defaults to [`DEFAULT_AUTO_UPDATE`].
    #[must_use]
    pub fn auto_update(&self) -> bool {
        self.auto_update.unwrap_or(DEFAULT_AUTO_UPDATE)
    }

    /// The installed extension ids. Defaults to empty.
    #[must_use]
    pub fn installed(&self) -> &[String] {
        self.installed.as_deref().unwrap_or(&[])
    }

    /// Sets `extensions.auto_update`. `None` removes the key.
    pub fn set_auto_update(&mut self, value: Option<bool>) -> &mut Self {
        self.auto_update = value;
        self
    }

    /// Sets `extensions.installed`. `None` removes the key.
    pub fn set_installed(&mut self, value: Option<Vec<String>>) -> &mut Self {
        self.installed = value;
        self
    }

    /// Keys present in the file that this build does not understand.
    #[must_use]
    pub fn unknown_fields(&self) -> &BTreeMap<String, Value> {
        &self.unknown
    }

    /// Whether the section carries nothing at all, known or unknown.
    ///
    /// Destructured for the reason [`LauncherConfig::is_empty`] gives.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        let Self {
            auto_update,
            installed,
            unknown,
        } = self;
        auto_update.is_none() && installed.is_none() && unknown.is_empty()
    }
}

/// The `input_server` section: the keyboard helper behind snippet keyword
/// expansion (`vicinae-input-server`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct InputServerConfig {
    /// Whether the helper runs. Off, snippet keywords do not expand; on, it
    /// is started and restarted after a crash (five times, backing off).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("default" = DEFAULT_INPUT_SERVER_ENABLED))]
    enabled: Option<bool>,

    /// Keys this build does not know about, preserved verbatim.
    #[serde(flatten)]
    unknown: BTreeMap<String, Value>,
}

/// Default for `input_server.enabled`, as the C++ `config::InputServer`.
pub const DEFAULT_INPUT_SERVER_ENABLED: bool = true;

impl InputServerConfig {
    /// Whether the helper runs. Defaults to [`DEFAULT_INPUT_SERVER_ENABLED`].
    #[must_use]
    pub fn enabled(&self) -> bool {
        self.enabled.unwrap_or(DEFAULT_INPUT_SERVER_ENABLED)
    }

    /// Sets `input_server.enabled`. `None` removes the key.
    pub fn set_enabled(&mut self, value: Option<bool>) -> &mut Self {
        self.enabled = value;
        self
    }

    /// Whether the section carries nothing at all, known or unknown.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        let Self { enabled, unknown } = self;
        enabled.is_none() && unknown.is_empty()
    }
}

/// A parsed `vicinae.json`.
///
/// [`Config::default`] is the fully-defaulted configuration and is what an empty file produces.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[schemars(title = "vicinae.json", extend("$id" = SCHEMA_URL))]
pub struct Config {
    /// The JSON Schema this file follows, for editors. Ignored by the launcher.
    #[serde(rename = "$schema", default, skip_serializing_if = "Option::is_none")]
    schema: Option<String>,
    /// Launcher window behaviour and appearance.
    #[serde(default, skip_serializing_if = "LauncherConfig::is_empty")]
    launcher: LauncherConfig,
    /// Extension management.
    #[serde(default, skip_serializing_if = "ExtensionsConfig::is_empty")]
    extensions: ExtensionsConfig,
    /// Root search settings per provider, keyed by provider id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    providers: Option<BTreeMap<String, RootProviderSettings>>,
    /// Root items pinned to the top of the empty search, as `provider:entrypoint` ids.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    favorites: Option<Vec<String>>,
    /// Root items offered when a search has no match, as `provider:entrypoint` ids.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    fallbacks: Option<Vec<String>>,
    /// The snippet keyword expander's keyboard helper.
    #[serde(default, skip_serializing_if = "InputServerConfig::is_empty")]
    input_server: InputServerConfig,

    /// Top level keys this build does not know about, preserved verbatim.
    #[serde(flatten)]
    unknown: BTreeMap<String, Value>,
}

impl Config {
    /// Root-manager settings using upstream's `provider:entrypoint` identities.
    /// Unknown provider and entrypoint fields remain in the serialized config.
    #[must_use]
    pub fn root_config(&self) -> crate::root_items::RootConfig {
        use crate::root_items::{ItemConfig, ProviderConfig, RootConfig};

        RootConfig {
            providers: self
                .providers
                .iter()
                .flatten()
                .map(|(id, provider)| {
                    (
                        id.clone(),
                        ProviderConfig {
                            enabled: provider.enabled,
                            entrypoints: provider
                                .entrypoints
                                .iter()
                                .flatten()
                                .map(|(id, item)| {
                                    (
                                        id.clone(),
                                        ItemConfig {
                                            enabled: item.enabled,
                                            alias: item.alias.clone(),
                                            shortcut: item.shortcut.clone(),
                                        },
                                    )
                                })
                                .collect(),
                        },
                    )
                })
                .collect(),
            favorites: self.favorite_ids(),
            fallbacks: self.fallbacks.clone().unwrap_or_default(),
        }
    }

    /// The favourites, in the order arranged: the file's `favorites`, or the
    /// default file's `["clipboard:history"]` when it sets none, each C++
    /// builtin id read as the Compass command it names
    /// ([`crate::commands::canonical_id`]). An empty list the user wrote stays
    /// empty.
    #[must_use]
    pub fn favorite_ids(&self) -> Vec<String> {
        match &self.favorites {
            Some(favorites) => favorites
                .iter()
                .map(|id| crate::commands::canonical_id(id))
                .collect(),
            None => DEFAULT_FAVORITES
                .iter()
                .map(|id| crate::commands::canonical_id(id))
                .collect(),
        }
    }

    /// Writes what the root row's panel changed about the item `id`
    /// ([`crate::root_items::apply_edit`]) into this file: the whole
    /// `favorites` list (so a default the file did not set is written out,
    /// as `mergeWithUser` does with the merged list), or the item's entry
    /// under `providers`. Returns whether anything changed.
    pub fn apply_root_edit(&mut self, id: &str, edit: &crate::root_items::RootEdit) -> bool {
        use crate::root_items::RootEdit;
        match edit {
            RootEdit::Favorite(_) | RootEdit::MoveFavorite { .. } => {
                let mut root = crate::root_items::RootConfig {
                    favorites: self.favorite_ids(),
                    ..crate::root_items::RootConfig::default()
                };
                let changed = crate::root_items::apply_edit(&mut root, id, edit);
                if changed {
                    self.favorites = Some(root.favorites);
                }
                changed
            }
            RootEdit::Alias(_) | RootEdit::Disable => {
                let Some((provider, entrypoint)) = crate::root_items::split_entrypoint_id(id)
                else {
                    return false;
                };
                let item = self
                    .providers
                    .get_or_insert_with(BTreeMap::new)
                    .entry(provider.to_owned())
                    .or_default()
                    .entrypoints
                    .get_or_insert_with(BTreeMap::new)
                    .entry(entrypoint.to_owned())
                    .or_default();
                match edit {
                    RootEdit::Alias(alias) => item.alias = Some(alias.clone()),
                    _ => item.enabled = Some(false),
                }
                true
            }
            RootEdit::ResetRanking => false,
        }
    }

    /// The fallback commands a query with no better answer offers: the
    /// file's `fallbacks`, or the default file's `["files:search"]` when it
    /// sets none. An empty list the user wrote stays empty.
    #[must_use]
    pub fn fallback_ids(&self) -> Vec<String> {
        self.fallbacks
            .clone()
            .unwrap_or_else(|| vec![crate::commands::SEARCH_FILES_FALLBACK_ID.to_owned()])
    }

    /// A provider's `preferences` object, as `providers.<id>.preferences`
    /// holds it; `None` when the file sets none, or sets something that is not
    /// an object.
    #[must_use]
    pub fn provider_preferences(&self, provider: &str) -> Option<&serde_json::Map<String, Value>> {
        self.providers
            .as_ref()?
            .get(provider)?
            .unknown
            .get("preferences")?
            .as_object()
    }

    /// A command's own `preferences` object, as
    /// `providers.<provider>.entrypoints.<entrypoint>.preferences` holds it;
    /// `None` when the file sets none, or sets something that is not an
    /// object.
    #[must_use]
    pub fn entrypoint_preferences(
        &self,
        provider: &str,
        entrypoint: &str,
    ) -> Option<&serde_json::Map<String, Value>> {
        self.providers
            .as_ref()?
            .get(provider)?
            .entrypoints
            .as_ref()?
            .get(entrypoint)?
            .unknown
            .get("preferences")?
            .as_object()
    }

    /// The `launcher` section.
    #[must_use]
    pub fn launcher(&self) -> &LauncherConfig {
        &self.launcher
    }

    /// The `launcher` section, mutably.
    pub fn launcher_mut(&mut self) -> &mut LauncherConfig {
        &mut self.launcher
    }

    /// The `input_server` section.
    #[must_use]
    pub fn input_server(&self) -> &InputServerConfig {
        &self.input_server
    }

    /// The `input_server` section, mutably.
    pub fn input_server_mut(&mut self) -> &mut InputServerConfig {
        &mut self.input_server
    }

    /// The `extensions` section.
    #[must_use]
    pub fn extensions(&self) -> &ExtensionsConfig {
        &self.extensions
    }

    /// The `extensions` section, mutably.
    pub fn extensions_mut(&mut self) -> &mut ExtensionsConfig {
        &mut self.extensions
    }

    /// Top level keys present in the file that this build does not understand.
    #[must_use]
    pub fn unknown_fields(&self) -> &BTreeMap<String, Value> {
        &self.unknown
    }

    /// `font.normal.family`, when it names a family: `auto` and `system`
    /// (and no value) mean the launcher picks, which here is the desktop's
    /// interface font.
    #[must_use]
    pub fn font_family(&self) -> Option<&str> {
        let family = self
            .unknown
            .get("font")?
            .get("normal")?
            .get("family")?
            .as_str()?
            .trim();
        (!family.is_empty() && family != "auto" && family != "system").then_some(family)
    }

    /// Sets one of a provider's preferences
    /// (`providers.<provider>.preferences.<key>`), keeping the others, as the
    /// C++ `setPreferenceValues` merges a patch.
    pub fn set_provider_preference(
        &mut self,
        provider: &str,
        key: &str,
        value: Value,
    ) -> &mut Self {
        let settings = self
            .providers
            .get_or_insert_with(BTreeMap::new)
            .entry(provider.to_owned())
            .or_default();
        let preferences = settings
            .unknown
            .entry("preferences".to_owned())
            .or_insert_with(|| Value::Object(serde_json::Map::new()));
        if !preferences.is_object() {
            *preferences = Value::Object(serde_json::Map::new());
        }
        if let Some(preferences) = preferences.as_object_mut() {
            preferences.insert(key.to_owned(), value);
        }
        self
    }

    /// Sets `font.normal.family`, keeping the rest of the `font` object (its
    /// `rendering` and `normal.size`), as "Set as vicinae font" merges it.
    pub fn set_font_family(&mut self, family: &str) -> &mut Self {
        if !matches!(self.unknown.get("font"), Some(Value::Object(_))) {
            self.unknown
                .insert("font".to_owned(), Value::Object(serde_json::Map::new()));
        }
        if let Some(Value::Object(font)) = self.unknown.get_mut("font") {
            let normal = font
                .entry("normal")
                .or_insert_with(|| Value::Object(serde_json::Map::new()));
            if !normal.is_object() {
                *normal = Value::Object(serde_json::Map::new());
            }
            if let Some(normal) = normal.as_object_mut() {
                normal.insert("family".to_owned(), Value::String(family.to_owned()));
            }
        }
        self
    }

    /// Parses `data`. An empty or whitespace-only input yields the default configuration.
    ///
    /// `path` is used only to build error messages.
    ///
    /// # Errors
    ///
    /// [`ConfigError::Parse`] when `data` is not a JSON object of the expected shape.
    pub fn parse(data: &str, path: &Path) -> Result<Config, ConfigError> {
        if data.trim().is_empty() {
            return Ok(Config::default());
        }

        serde_json::from_str(data).map_err(|source| ConfigError::Parse {
            path: path.to_path_buf(),
            line: source.line(),
            column: source.column(),
            message: parse_message(&source),
            source,
        })
    }

    /// Loads the configuration from `path`.
    ///
    /// A missing file is not an error: it yields the default configuration, which is what a fresh
    /// install has.
    ///
    /// # Errors
    ///
    /// [`ConfigError::Read`] if the file exists but cannot be read, [`ConfigError::Parse`] if it
    /// is not valid.
    pub fn load_from(path: impl AsRef<Path>) -> Result<Config, ConfigError> {
        let path = path.as_ref();
        let data = match std::fs::read_to_string(path) {
            Ok(data) => data,
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
                tracing::debug!(path = %path.display(), "no configuration file, using defaults");
                return Ok(Config::default());
            }
            Err(source) => {
                return Err(ConfigError::Read {
                    path: path.to_path_buf(),
                    source,
                });
            }
        };

        Config::parse(&data, path)
    }

    /// Loads the configuration from [`default_config_path`].
    ///
    /// When there is no `vicinae.json` yet but the C++ engine's `settings.json` sits beside it,
    /// that file is migrated in memory, so someone switching engines keeps their settings before
    /// anything has been written. See [`Config::load_or_migrate`].
    ///
    /// Prefer [`Config::load_from`] anywhere the path should be injectable — notably in tests,
    /// which must never touch the invoking user's real configuration.
    ///
    /// # Errors
    ///
    /// See [`Config::load_from`], plus [`ConfigError::NoConfigDir`].
    pub fn load() -> Result<Config, ConfigError> {
        let path = default_config_path()?;
        let legacy = crate::config_migration::legacy_config_path().ok();
        Config::load_or_migrate(&path, legacy.as_deref())
    }

    /// Loads `path`, or, when it does not exist, migrates `legacy` (the C++ `settings.json`).
    ///
    /// Nothing is written: the migrated configuration is only materialised when something saves
    /// it, and `vicinae config migrate --write` does that on purpose. A legacy file that cannot be
    /// migrated is logged and the defaults are used, since it is not this engine's file to reject.
    ///
    /// # Errors
    ///
    /// See [`Config::load_from`]; the legacy file never produces an error.
    pub fn load_or_migrate(path: &Path, legacy: Option<&Path>) -> Result<Config, ConfigError> {
        let legacy = legacy.filter(|legacy| !path.exists() && legacy.is_file());
        let Some(legacy) = legacy else {
            return Config::load_from(path);
        };
        match crate::config_migration::migrate_file(legacy) {
            Ok(migration) => {
                tracing::info!(
                    from = %legacy.display(),
                    mapped = migration.mapped.len(),
                    skipped = migration.skipped.len(),
                    "no vicinae.json; using the settings migrated from the C++ engine"
                );
                Ok(migration.config)
            }
            Err(error) => {
                tracing::warn!(%error, "could not migrate the C++ engine's settings; using defaults");
                Ok(Config::default())
            }
        }
    }

    /// The `$schema` the file names, if any.
    #[must_use]
    pub fn schema(&self) -> Option<&str> {
        self.schema.as_deref()
    }

    /// Sets `$schema`. `None` removes the key.
    pub fn set_schema(&mut self, value: Option<String>) -> &mut Self {
        self.schema = value;
        self
    }

    /// Serialises the configuration, unknown fields included.
    ///
    /// # Errors
    ///
    /// Only if a preserved unknown value cannot be serialised, which cannot happen for values
    /// that came from [`Config::parse`].
    pub fn to_json_pretty(&self) -> Result<String, serde_json::Error> {
        let mut out = serde_json::to_string_pretty(self)?;
        out.push('\n');
        Ok(out)
    }

    /// Writes the configuration to `path`, creating parent directories.
    ///
    /// The write goes to a sibling temporary file and is renamed into place, so a crash cannot
    /// leave a half-written config behind.
    ///
    /// # Errors
    ///
    /// [`ConfigError::Write`] on any I/O failure.
    pub fn save_to(&self, path: impl AsRef<Path>) -> Result<(), ConfigError> {
        let path = path.as_ref();
        let data = self.to_json_pretty().map_err(|err| ConfigError::Write {
            path: path.to_path_buf(),
            source: std::io::Error::other(err),
        })?;
        crate::atomic_write(path, data.as_bytes()).map_err(|source| ConfigError::Write {
            path: path.to_path_buf(),
            source,
        })
    }
}

/// Turns a [`serde_json::Error`] into a message that names the problem without repeating the
/// position, which the [`ConfigError::Parse`] display already carries.
fn parse_message(err: &serde_json::Error) -> String {
    let text = err.to_string();
    match text.find(" at line ") {
        Some(at) => text[..at].to_owned(),
        None => text,
    }
}

/// The JSON Schema for `vicinae.json`, generated from [`Config`].
///
/// This is what `packaging/schema/vicinae.schema.json` holds. Regenerate the committed copy with
/// `COMPASS_UPDATE_SCHEMA=1 cargo test -p compass-core --test config_schema`, or print it with
/// `vicinae config schema`.
#[must_use]
pub fn json_schema() -> Value {
    schemars::schema_for!(Config).to_value()
}

/// [`json_schema`] as the pretty-printed, newline-terminated text that is committed.
#[must_use]
pub fn json_schema_pretty() -> String {
    let mut out = serde_json::to_string_pretty(&json_schema()).unwrap_or_default();
    out.push('\n');
    out
}

/// The configuration every unset key amounts to, as `vicinae config default`
/// prints it: each `default` the [`json_schema`] documents, nested as the
/// file nests it, plus the fallbacks and the `$schema` line.
///
/// Read from the schema rather than written out a second time, so a default
/// changed in one place cannot be printed stale from another.
#[must_use]
pub fn default_document() -> Value {
    fn defaults(node: &Value, defs: &Value) -> Option<Value> {
        if let Some(default) = node.get("default") {
            return Some(default.clone());
        }
        if let Some(name) = node
            .get("$ref")
            .and_then(Value::as_str)
            .and_then(|r| r.rsplit('/').next())
        {
            return defaults(defs.get(name)?, defs);
        }
        let properties = node.get("properties")?.as_object()?;
        let object: serde_json::Map<String, Value> = properties
            .iter()
            .filter_map(|(key, property)| Some((key.clone(), defaults(property, defs)?)))
            .collect();
        (!object.is_empty()).then_some(Value::Object(object))
    }
    let schema = json_schema();
    let defs = schema.get("$defs").cloned().unwrap_or(Value::Null);
    let mut document = serde_json::Map::new();
    document.insert("$schema".to_owned(), Value::String(SCHEMA_URL.to_owned()));
    if let Some(Value::Object(found)) = defaults(&schema, &defs) {
        document.extend(found);
    }
    document.insert(
        "fallbacks".to_owned(),
        serde_json::json!(Config::default().fallback_ids()),
    );
    Value::Object(document)
}

/// Overrides where `vicinae.json` is read and written, as the C++ server's
/// `--config`; `vicinae server --config` sets it for the engine it starts.
pub const CONFIG_PATH_ENV: &str = "COMPASS_CONFIG";

/// `$XDG_CONFIG_HOME/vicinae/vicinae.json`, falling back to `~/.config`; or
/// [`CONFIG_PATH_ENV`] when that is set.
///
/// # Errors
///
/// [`ConfigError::NoConfigDir`] when neither `$XDG_CONFIG_HOME` nor `$HOME` is usable.
pub fn default_config_path() -> Result<PathBuf, ConfigError> {
    if let Some(path) = std::env::var_os(CONFIG_PATH_ENV).filter(|path| !path.is_empty()) {
        return Ok(PathBuf::from(path));
    }
    let dir = dirs::config_dir().ok_or(ConfigError::NoConfigDir)?;
    Ok(dir.join(CONFIG_RELATIVE_PATH))
}
