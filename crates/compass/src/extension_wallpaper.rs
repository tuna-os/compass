//! The wallpaper an extension sets through `Wallpaper/set`.
//!
//! `WallpaperManager` and its six Linux backends, run: the plans are
//! `compass_core::wallpaper`'s (which command, which schema, which word for
//! each fit); this finds the backend the way the C++ does and carries the
//! plan out. Running daemons come first — hyprpaper answering
//! `hyprctl hyprpaper listactive`, swww (or awww) answering `query` — then
//! the desktops: GNOME, Cinnamon and MATE by name with `gsettings` on the
//! path, KDE by `org.kde.plasmashell` on the session bus.
//!
//! The backend is resolved once per engine, as `m_resolved` keeps it. Every
//! program runs on the host (`flatpak-spawn --host` inside the Flatpak).

use std::path::Path;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use compass_core::wallpaper::{
    Backend, CANDIDATE_ORDER, Command, PLASMA_SERVICE, SWWW_BINARIES, UNSUPPORTED_MESSAGE,
    WallpaperFit, WallpaperRequest,
};
use compass_worker_host::wallpaper_service::{Fit, Request, Wallpaper};

/// How long one command may run; a ceiling, as the C++'s is, for a slow
/// apply (a large animated image through swww).
const COMMAND_TIMEOUT: Duration = Duration::from_secs(30);

static RESOLVED: OnceLock<Option<Backend>> = OnceLock::new();

/// [`Wallpaper`] on this desktop.
#[derive(Debug, Clone, Default)]
pub struct EngineWallpaper {
    handle: Option<tokio::runtime::Handle>,
}

impl EngineWallpaper {
    /// Reaches the session bus (for KDE) on `handle`, when there is one.
    #[must_use]
    pub const fn new(handle: Option<tokio::runtime::Handle>) -> Self {
        Self { handle }
    }

    /// The backend this desktop uses, resolved on first use. Blocking.
    #[must_use]
    pub fn backend(&self) -> Option<Backend> {
        *RESOLVED.get_or_init(|| {
            let found = CANDIDATE_ORDER
                .iter()
                .copied()
                .find(|backend| self.is_activatable(*backend));
            tracing::info!(backend = ?found, "wallpaper backend");
            found
        })
    }

    /// `WallpaperManager::canSetWallpaper`, for the command's capabilities.
    #[must_use]
    pub fn can_set(&self) -> bool {
        self.backend().is_some()
    }

    fn is_activatable(&self, backend: Backend) -> bool {
        let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
        let gdm = std::env::var("GDMSESSION").unwrap_or_default();
        match backend {
            Backend::Hyprpaper => {
                on_path("hyprctl")
                    && run(&Command {
                        program: "hyprctl".into(),
                        args: vec!["hyprpaper".into(), "listactive".into()],
                        may_fail: false,
                    })
                    .is_ok()
            }
            Backend::Swww => swww_binary().is_some_and(|binary| {
                run(&Command {
                    program: binary.to_owned(),
                    args: vec!["query".into()],
                    may_fail: false,
                })
                .is_ok()
            }),
            Backend::Kde => self.plasma_running(),
            Backend::Gnome | Backend::Cinnamon | Backend::Mate => {
                compass_core::wallpaper::desktop_matches(backend, &desktop, &gdm).unwrap_or(false)
                    && on_path("gsettings")
            }
        }
    }

    fn plasma_running(&self) -> bool {
        let Some(handle) = &self.handle else {
            return false;
        };
        handle.block_on(async {
            let Ok(connection) = zbus::Connection::session().await else {
                return false;
            };
            let Ok(bus) = zbus::fdo::DBusProxy::new(&connection).await else {
                return false;
            };
            let Ok(name) = zbus::names::BusName::try_from(PLASMA_SERVICE) else {
                return false;
            };
            bus.name_has_owner(name).await.unwrap_or(false)
        })
    }

    fn set_on_kde(&self, request: &WallpaperRequest) -> Result<(), String> {
        let Some(handle) = &self.handle else {
            return Err(UNSUPPORTED_MESSAGE.to_owned());
        };
        let script = compass_core::wallpaper::kde_script(request);
        handle.block_on(async move {
            let connection = zbus::Connection::session()
                .await
                .map_err(|err| err.to_string())?;
            connection
                .call_method(
                    Some(PLASMA_SERVICE),
                    "/PlasmaShell",
                    Some("org.kde.PlasmaShell"),
                    "evaluateScript",
                    &(script,),
                )
                .await
                .map(drop)
                .map_err(|err| err.to_string())
        })
    }
}

impl Wallpaper for EngineWallpaper {
    fn set(&self, request: &Request) -> Result<(), String> {
        let request = WallpaperRequest {
            path: request.path.clone(),
            screen: request.screen.clone(),
            fit: match request.fit {
                Fit::Cover => WallpaperFit::Cover,
                Fit::Contain => WallpaperFit::Contain,
                Fit::Stretch => WallpaperFit::Stretch,
                Fit::Center => WallpaperFit::Center,
                Fit::Tile => WallpaperFit::Tile,
            },
        };
        let backend = self
            .backend()
            .ok_or_else(|| UNSUPPORTED_MESSAGE.to_owned())?;
        if !Path::new(&request.path).is_file() {
            return Err(compass_core::wallpaper::no_such_file_message(&request.path));
        }
        let commands = match backend {
            Backend::Kde => return self.set_on_kde(&request),
            Backend::Hyprpaper => vec![compass_core::wallpaper::hyprpaper_command(&request)],
            Backend::Swww => {
                let binary = swww_binary().ok_or("neither awww nor swww is installed")?;
                vec![compass_core::wallpaper::swww_command(binary, &request)]
            }
            Backend::Gnome => compass_core::wallpaper::gnome_commands(&request),
            Backend::Cinnamon => compass_core::wallpaper::cinnamon_commands(&request),
            Backend::Mate => compass_core::wallpaper::mate_commands(&request),
        };
        for command in &commands {
            if let Err(err) = run(command)
                && !command.may_fail
            {
                return Err(err);
            }
        }
        Ok(())
    }
}

/// `awww`, else `swww`, whichever is on the path.
fn swww_binary() -> Option<&'static str> {
    SWWW_BINARIES.iter().copied().find(|name| on_path(name))
}

/// `QStandardPaths::findExecutable`: `program` is an executable file in a
/// `$PATH` directory. Inside the Flatpak the host's path is not visible, so
/// there a program counts as present and running it decides.
fn on_path(program: &str) -> bool {
    if Path::new("/.flatpak-info").exists() {
        return true;
    }
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|dir| {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(dir.join(program))
            .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
    })
}

/// `wallpaper::runCommand`: runs `command` on the host and waits, with the
/// C++'s messages for a program that will not start, one that overruns, and
/// one that fails (its stderr, else its exit code).
fn run(command: &Command) -> Result<(), String> {
    let mut child = compass_platform_linux::host_command(&command.program)
        .args(&command.args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|err| format!("failed to start {}: {err}", command.program))?;
    let deadline = Instant::now() + COMMAND_TIMEOUT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("{} timed out", command.program));
            }
            Err(err) => return Err(format!("{}: {err}", command.program)),
        }
    };
    if status.success() {
        return Ok(());
    }
    let mut stderr = String::new();
    if let Some(mut pipe) = child.stderr.take() {
        use std::io::Read;
        let _ = pipe.read_to_string(&mut stderr);
    }
    let stderr = stderr.trim();
    Err(if stderr.is_empty() {
        format!(
            "{} exited with code {}",
            command.program,
            status.code().unwrap_or(-1)
        )
    } else {
        stderr.to_owned()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_failing_command_says_its_stderr_else_its_code() {
        let failed = run(&Command {
            program: "sh".into(),
            args: vec!["-c".into(), "echo 'No such schema' >&2; exit 1".into()],
            may_fail: false,
        });
        assert_eq!(failed, Err("No such schema".to_owned()));
        let failed = run(&Command {
            program: "sh".into(),
            args: vec!["-c".into(), "exit 3".into()],
            may_fail: false,
        });
        assert_eq!(failed, Err("sh exited with code 3".to_owned()));
        let missing = run(&Command {
            program: "compass-no-such-program".into(),
            args: Vec::new(),
            may_fail: false,
        });
        assert!(
            missing.is_err_and(|err| err.starts_with("failed to start compass-no-such-program"))
        );
    }
}
