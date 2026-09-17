//! Session environment, XDG paths, runtime directory, and execution environment checks.

use std::path::{Path, PathBuf};
use compass_ipc::{DoctorCheck, DoctorStatus, SocketPath};
use super::check;
use super::env::Env;
use super::fs::FsProbe;
use crate::engine::Engine;

/// Marker file the Flatpak runtime places in every sandbox.
pub const FLATPAK_INFO_PATH: &str = "/.flatpak-info";

/// Which engine this invocation selected, and whether this binary can serve it.
#[must_use]
pub fn engine(selected: Engine) -> DoctorCheck {
    match selected {
        Engine::Rust => check(
            "engine.selected",
            DoctorStatus::Ok,
            "rust — served by this binary",
        ),
        Engine::Cpp => check(
            "engine.selected",
            DoctorStatus::Warn,
            "cpp — this binary is the Rust engine and cannot dispatch to the C++ one \
             (PLAN.md §5; the dispatching front-end lands with the Phase 7 cutover). \
             Commands that need the engine will refuse; run the C++ `vicinae` directly, \
             or pass --engine rust / COMPASS_ENGINE=rust",
        ),
    }
}

/// Wayland, X11, or no graphical session at all.
#[must_use]
pub fn session_type(env: &Env) -> DoctorCheck {
    const NAME: &str = "session.type";
    match (env.get("WAYLAND_DISPLAY"), env.get("DISPLAY")) {
        (Some(wayland), Some(x11)) => check(
            NAME,
            DoctorStatus::Ok,
            format!(
                "Wayland (WAYLAND_DISPLAY={wayland}), with XWayland available at DISPLAY={x11}"
            ),
        ),
        (Some(wayland), None) => check(
            NAME,
            DoctorStatus::Ok,
            format!("Wayland (WAYLAND_DISPLAY={wayland})"),
        ),
        (None, Some(x11)) => check(
            NAME,
            DoctorStatus::Warn,
            format!(
                "X11 only (DISPLAY={x11}, WAYLAND_DISPLAY unset). The Rust engine targets \
                 Wayland; the X11 hotkey backend is Phase 5 work, so the global hotkey will \
                 not bind here"
            ),
        ),
        (None, None) => check(
            NAME,
            DoctorStatus::Fail,
            "no graphical session: neither WAYLAND_DISPLAY nor DISPLAY is set. The CLI and \
             this diagnostic work, but the launcher window cannot be opened",
        ),
    }
}

/// `XDG_RUNTIME_DIR`, and whether the socket fell back because of its absence.
#[must_use]
pub fn runtime_dir(env: &Env, socket: &SocketPath) -> DoctorCheck {
    const NAME: &str = "xdg.runtime-dir";
    match env.get("XDG_RUNTIME_DIR") {
        Some(dir) => check(NAME, DoctorStatus::Ok, format!("XDG_RUNTIME_DIR={dir}")),
        None if socket.is_fallback() => check(
            NAME,
            DoctorStatus::Warn,
            format!(
                "XDG_RUNTIME_DIR is unset, so the socket falls back to {socket}. That path is \
                 not tmpfs-backed, survives logout and lives under a world-readable /tmp. \
                 Common in bare ssh sessions, cron and minimal containers; inside a normal \
                 desktop session it means the session manager did not set it up"
            ),
        ),
        None => check(
            NAME,
            DoctorStatus::Warn,
            format!(
                "XDG_RUNTIME_DIR is unset. The socket path was set explicitly ({socket}), so \
                 the /tmp fallback is not in use, but anything else resolving runtime state \
                 from the environment will still fall back"
            ),
        ),
    }
}

/// Where the IPC socket is, and whether an engine is answering on it.
#[must_use]
pub fn ipc_socket(socket: &SocketPath, file_exists: bool, listening: bool) -> DoctorCheck {
    const NAME: &str = "ipc.socket";
    match (listening, file_exists) {
        (true, _) => check(
            NAME,
            DoctorStatus::Ok,
            format!("an engine is listening on {socket}"),
        ),
        (false, true) => check(
            NAME,
            DoctorStatus::Warn,
            format!(
                "{socket} exists but nothing accepts connections on it: a stale socket left by \
                 a crashed engine. The next engine start reclaims it automatically"
            ),
        ),
        (false, false) => check(
            NAME,
            DoctorStatus::Warn,
            format!(
                "no engine running: {socket} does not exist. Start the engine, then retry; \
                 `vicinae toggle`, `show`, `hide` and `ping` all need it"
            ),
        ),
    }
}

/// Whether we are running inside a Flatpak sandbox.
///
/// Informational either way — both answers are supported configurations — but
/// which one it is changes how apps get launched and what has to be in the
/// manifest, so a bug report needs it stated.
pub fn flatpak<F: FsProbe>(fs: &F) -> DoctorCheck {
    const NAME: &str = "flatpak.sandbox";
    if fs.exists(Path::new(FLATPAK_INFO_PATH)) {
        check(
            NAME,
            DoctorStatus::Ok,
            format!(
                "inside a Flatpak sandbox ({FLATPAK_INFO_PATH} present). Host applications are \
                 launched through `flatpak-spawn --host` with an OpenURI fallback, and \
                 .desktop/icon indexing needs --filesystem=host-os:ro plus read access to \
                 ~/.local/share/{{applications,icons}}"
            ),
        )
    } else {
        check(
            NAME,
            DoctorStatus::Ok,
            format!(
                "not sandboxed ({FLATPAK_INFO_PATH} absent); applications are launched directly"
            ),
        )
    }
}

/// The XDG application directories this session searches, in order.
#[must_use]
pub fn application_dir_paths(env: &Env) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();

    let data_home = env
        .get("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| env.get("HOME").map(|h| Path::new(h).join(".local/share")));
    if let Some(home) = data_home {
        dirs.push(home.join("applications"));
    }

    let data_dirs = env.list("XDG_DATA_DIRS");
    let data_dirs: Vec<&str> = if data_dirs.is_empty() {
        vec!["/usr/local/share", "/usr/share"]
    } else {
        data_dirs
    };
    for dir in data_dirs {
        dirs.push(Path::new(dir).join("applications"));
    }

    // Order-preserving dedupe: XDG_DATA_DIRS routinely repeats an entry, and
    // counting the same directory twice would inflate the .desktop total.
    let mut seen = std::collections::BTreeSet::new();
    dirs.retain(|dir| seen.insert(dir.clone()));
    dirs
}

/// Which application directories exist, are readable, and how much is in them.
pub fn application_dirs<F: FsProbe>(env: &Env, fs: &F) -> DoctorCheck {
    const NAME: &str = "xdg.application-dirs";

    let dirs = application_dir_paths(env);
    if dirs.is_empty() {
        return check(
            NAME,
            DoctorStatus::Fail,
            "no application directories resolve: neither XDG_DATA_HOME nor HOME is set and \
             XDG_DATA_DIRS is empty. App search will return nothing",
        );
    }

    let mut lines = Vec::new();
    let mut total = 0usize;
    let mut unreadable = 0usize;

    for dir in &dirs {
        if !fs.exists(dir) {
            lines.push(format!("{} — absent", dir.display()));
            continue;
        }
        match fs.dir_entries(dir) {
            Ok(entries) => {
                let count = entries.iter().filter(|e| e.ends_with(".desktop")).count();
                total += count;
                lines.push(format!("{} — {count} .desktop files", dir.display()));
            }
            Err(err) => {
                unreadable += 1;
                lines.push(format!("{} — unreadable: {err}", dir.display()));
            }
        }
    }

    let status = if total == 0 {
        DoctorStatus::Fail
    } else if unreadable > 0 {
        DoctorStatus::Warn
    } else {
        DoctorStatus::Ok
    };

    let headline = match status {
        DoctorStatus::Ok => format!("{total} .desktop files across {} directories", dirs.len()),
        DoctorStatus::Warn => format!(
            "{total} .desktop files, but {unreadable} of {} directories could not be read, so \
             some applications will be missing from search",
            dirs.len()
        ),
        DoctorStatus::Fail => format!(
            "no .desktop files found in any of the {} application directories; app search will \
             return nothing",
            dirs.len()
        ),
    };

    check(NAME, status, format!("{headline}\n{}", lines.join("\n")))
}
