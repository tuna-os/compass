//! Anonymous telemetry: what is sent, how often, and what identifies it.
//!
//! A port of `TelemetryService` (`src/server/src/services/telemetry/`) and the
//! `Environment` helpers it reads, minus the HTTP client and the process
//! timer.
//!
//! # This is opt-in, and stays opt-in
//!
//! Nothing here sends anything until [`TelemetryService::set_enabled`] is
//! called with `true`, which in the C++ happens only from the configuration.
//! The port keeps that: a freshly constructed service is disabled, and
//! [`TelemetryService::try_send_system_info`] is a no-op while it is. Sending
//! goes through the [`Transport`] trait, so nothing in this crate opens a
//! socket and the tests do not either.
//!
//! # What is worth pinning is the shape of the record
//!
//! The payload's field names, which fields are lowercased and which are not,
//! and the once-a-day rule are all observable in the data that arrives at the
//! other end. Getting them wrong does not fail anything locally — it quietly
//! makes two engines' records incomparable.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// How long between system-info records, from `ONE_DAY_SECS`.
pub const SYSTEM_INFO_INTERVAL_SECS: u64 = 86_400;

/// How often the C++ timer wakes up to check, in seconds — `1h`.
///
/// Shorter than the interval on purpose: waking daily would drift by however
/// long the machine was asleep.
pub const POLL_INTERVAL_SECS: u64 = 3600;

/// The file the state is kept in, under the state directory.
pub const STATE_FILE: &str = "telemetry.json";

/// Where records are posted, from `Environment::vicinaeApiBaseUrl`.
pub const DEFAULT_API_BASE_URL: &str = "https://api.vicinae.com/v1";

/// The environment variable that overrides [`DEFAULT_API_BASE_URL`].
pub const API_URL_ENV: &str = "VICINAE_API_URL";

/// The system-info endpoint, relative to the base URL.
pub const SYSTEM_INFO_PATH: &str = "/telemetry/system-info";

/// The endpoint that unlinks a user id from past records.
pub const FORGET_PATH: &str = "/telemetry/forget";

/// What the C++ prints when telemetry is switched on.
pub const DOC_TELEMETRY_URL: &str = "https://docs.vicinae.com/telemetry";

/// The base URL to post to, honouring [`API_URL_ENV`].
#[must_use]
pub fn api_base_url(env_value: Option<&str>) -> String {
    env_value
        .filter(|url| !url.is_empty())
        .unwrap_or(DEFAULT_API_BASE_URL)
        .to_owned()
}

/// One screen, as the record describes it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScreenInfo {
    /// Its size in pixels.
    pub resolution: Resolution,
    /// Its device pixel ratio.
    pub scale: f64,
}

/// A screen's pixel size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Resolution {
    /// Width in pixels.
    pub width: i32,
    /// Height in pixels.
    pub height: i32,
}

/// The system-info record.
///
/// The field names are the C++ member names, because glaze serialises members
/// as they are written — no snake_case adapter is registered for this struct,
/// unlike the pactl ones. A record with different key names is a record the
/// server files separately.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SystemInfoRequest {
    /// The anonymous id.
    pub user_id: String,
    /// `XDG_CURRENT_DESKTOP`, split — lowercased.
    pub desktops: Vec<String>,
    /// The build's version — lowercased.
    pub vicinae_version: String,
    /// The Qt version it was built against.
    pub qt_version: String,
    /// `wayland` or `xcb`.
    pub display_protocol: String,
    /// The CPU architecture — lowercased.
    pub architecture: String,
    /// The kernel type.
    pub operating_system: String,
    /// How the build was produced — lowercased.
    pub build_provenance: String,
    /// The system locale.
    pub locale: String,
    /// Every screen.
    pub screens: Vec<ScreenInfo>,
    /// Desktop, laptop or other.
    pub chassis_type: String,
    /// The kernel version.
    pub kernel_version: String,
    /// The distribution.
    pub product_id: String,
    /// The distribution's version.
    pub product_version: String,
}

/// What is kept on disk between runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct State {
    /// The anonymous id, generated once and kept.
    pub user_id: String,
    /// When system info last went out, in seconds since the epoch.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_info_last_sent_at: Option<u64>,
}

impl SystemInfoRequest {
    /// An empty record, usable from a `const fn`.
    #[must_use]
    pub const fn new_const() -> Self {
        Self {
            user_id: String::new(),
            desktops: Vec::new(),
            vicinae_version: String::new(),
            qt_version: String::new(),
            display_protocol: String::new(),
            architecture: String::new(),
            operating_system: String::new(),
            build_provenance: String::new(),
            locale: String::new(),
            screens: Vec::new(),
            chassis_type: String::new(),
            kernel_version: String::new(),
            product_id: String::new(),
            product_version: String::new(),
        }
    }
}

/// Build the anonymous id from a UUID, as `generateUserId` does.
///
/// The `user-` prefix is what makes a record recognisable as ours in a log
/// that carries other kinds of id.
#[must_use]
pub fn user_id_from_uuid(uuid: &str) -> String {
    format!("user-{uuid}")
}

/// Lowercase, as `TelemetryService::toLower` does.
///
/// # Only some fields get this
///
/// The C++ lowercases `architecture`, `buildProvenance`, `vicinaeVersion` and
/// each of `desktops`, and leaves `displayProtocol`, `locale`,
/// `operatingSystem`, `chassisType`, `kernelVersion`, `productId` and
/// `productVersion` as they came. The asymmetry looks accidental, but it is
/// what the collected data is shaped like, so normalising the rest would make
/// this engine's records group differently from the C++'s.
#[must_use]
pub fn to_lower(text: &str) -> String {
    text.to_lowercase()
}

/// Which distribution to report.
///
/// # Omarchy is looked for by directory, not by `/etc/os-release`
///
/// It does not override `os-release`, so it would otherwise be counted as
/// Arch. The C++ comment says why that matters: a lot of its users come from
/// macOS expecting a Raycast replacement, which makes them a distinct
/// audience worth seeing separately.
#[must_use]
pub fn determine_product_id(data_home: &Path, product_type: &str) -> String {
    if data_home.join("omarchy").is_dir() {
        return "omarchy".to_owned();
    }
    product_type.to_owned()
}

/// The chassis codes `/sys/class/dmi/id/chassis_type` reports as a desktop.
pub const DESKTOP_CHASSIS_CODES: &[i32] = &[3, 4, 5, 6, 7];

/// The chassis codes reported as a laptop.
pub const LAPTOP_CHASSIS_CODES: &[i32] = &[8, 9, 10, 11, 12, 13, 14, 30, 31, 32];

/// Classify a DMI chassis code, as `Environment::chassisType` does.
///
/// An unreadable file is `"unknown"`, which is deliberately not `"other"`: the
/// difference between "this machine is neither" and "we could not tell" is the
/// difference between a data point and a gap.
#[must_use]
pub fn chassis_type(code: Option<i32>) -> &'static str {
    let Some(code) = code else {
        return "unknown";
    };
    if DESKTOP_CHASSIS_CODES.contains(&code) {
        "desktop"
    } else if LAPTOP_CHASSIS_CODES.contains(&code) {
        "laptop"
    } else {
        "other"
    }
}

/// Where a record goes.
pub trait Transport {
    /// Post a system-info record. `true` if the server accepted it.
    fn post_system_info(&self, request: &SystemInfoRequest) -> bool;

    /// Ask for `user_id` to be stripped from past records.
    fn post_forget(&self, user_id: &str) -> bool;
}

/// Read the state file at `path`, creating one if it is not there.
///
/// # A corrupt file keeps its id rather than inventing one
///
/// The C++ warns and carries on with whatever glaze left behind, which for an
/// unreadable file is a `State` with an *empty* `userId` — so every record
/// after that is filed under `""`. This generates a fresh id instead, which
/// keeps the records attributable to *a* machine, and writes it back so the
/// next run is stable again.
///
/// # Errors
///
/// Returns the io error if the state cannot be written.
pub fn load_state(path: &Path, new_uuid: impl FnOnce() -> String) -> std::io::Result<State> {
    let stored = path
        .is_file()
        .then(|| fs::read_to_string(path).ok())
        .flatten()
        .and_then(|text| serde_json::from_str::<State>(&text).ok())
        .filter(|state| !state.user_id.is_empty());

    if let Some(state) = stored {
        return Ok(state);
    }

    let state = State {
        user_id: user_id_from_uuid(&new_uuid()),
        system_info_last_sent_at: None,
    };
    save_state(path, &state)?;
    Ok(state)
}

/// Write `state` to `path`, creating the directory above it.
///
/// # Errors
///
/// Returns the io error if the directory or file cannot be written.
pub fn save_state(path: &Path, state: &State) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string(state)
        .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err))?;
    fs::write(path, json)
}

/// The path the state lives at, under a state directory.
#[must_use]
pub fn state_path(state_dir: &Path) -> PathBuf {
    state_dir.join(STATE_FILE)
}

/// Whether a record is due at `now`.
#[must_use]
pub const fn should_send_system_info(state: &State, now: u64) -> bool {
    match state.system_info_last_sent_at {
        None => true,
        Some(last) => now.saturating_sub(last) >= SYSTEM_INFO_INTERVAL_SECS,
    }
}

/// The telemetry service: enabled or not, and due or not.
#[derive(Debug)]
pub struct TelemetryService<T> {
    /// Where records go.
    transport: T,
    /// The state, mirrored from disk.
    state: State,
    /// Whether telemetry is on. A new service is off.
    ///
    /// # A divergence from a `static` that should not have been one
    ///
    /// `TelemetryService::setEnabled` keeps its previous value in a
    /// function-local `static`, so it is shared by every instance in the
    /// process and survives the service being destroyed. With one service per
    /// process that is invisible; with two, the second one's first
    /// `setEnabled(true)` is swallowed as "no change" and telemetry silently
    /// never starts. This is per-instance, which is what the code reads as
    /// though it were.
    enabled: Option<bool>,
    /// The machine description future records carry, which the caller fills
    /// in: the C++ reads it off `QGuiApplication` and `QSysInfo`, neither of
    /// which exists here.
    pending: SystemInfoRequest,
}

impl<T: Transport> TelemetryService<T> {
    /// A disabled service over `transport`, holding `state`.
    pub const fn new(transport: T, state: State) -> Self {
        Self {
            transport,
            state,
            enabled: None,
            pending: SystemInfoRequest::new_const(),
        }
    }

    /// The state as it stands.
    #[must_use]
    pub const fn state(&self) -> &State {
        &self.state
    }

    /// The transport, for a caller that has to read back what was sent.
    pub const fn transport(&self) -> &T {
        &self.transport
    }

    /// Whether telemetry is currently on.
    #[must_use]
    pub fn is_enabled(&self) -> bool {
        self.enabled.unwrap_or(false)
    }

    /// Switch telemetry on or off.
    ///
    /// Returns whether this changed anything. Switching it *on* sends a record
    /// immediately if one is due, which is what makes the first day's data
    /// appear without waiting an hour for the timer.
    pub fn set_enabled(&mut self, enabled: bool, now: u64) -> bool {
        if self.enabled == Some(enabled) {
            return false;
        }
        self.enabled = Some(enabled);
        // Unconditional, and a no-op when switching off, because
        // `try_send_system_info` checks `is_enabled` itself. The C++ branches
        // here because its two arms start and stop a timer; with no timer to
        // own there is only one rule left, and guarding it twice would make
        // the guard untestable.
        self.try_send_system_info(now);
        true
    }

    /// Send a record if telemetry is on and one is due.
    ///
    /// Returns whether one went out and was accepted. A rejected record does
    /// *not* update the timestamp, so the next check tries again rather than
    /// waiting another day on a record nobody received.
    pub fn try_send_system_info(&mut self, now: u64) -> bool {
        if !self.is_enabled() {
            return false;
        }
        if !should_send_system_info(&self.state, now) {
            return false;
        }
        self.send_system_info(now)
    }

    /// Send a record unconditionally.
    fn send_system_info(&mut self, now: u64) -> bool {
        let mut request = self.build_request();
        request.user_id.clone_from(&self.state.user_id);

        if !self.transport.post_system_info(&request) {
            return false;
        }
        self.state.system_info_last_sent_at = Some(now);
        true
    }

    /// The record's non-identifying half, which the caller fills in.
    fn build_request(&self) -> SystemInfoRequest {
        self.pending.clone()
    }

    /// Set the machine description future records carry.
    pub fn set_system_info(&mut self, request: SystemInfoRequest) {
        self.pending = request;
    }

    /// Ask the server to unlink this machine's id from past records.
    pub fn forget(&self) -> bool {
        self.transport.post_forget(&self.state.user_id)
    }
}
