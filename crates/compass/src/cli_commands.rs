//! The C++ CLI's remaining subcommands: `version`, `server`, `cmd`, `app`,
//! `fs`, `script`, `state`, `logs`, and `theme template`/`check`/`paths`.
//!
//! Each follows its C++ counterpart in `src/cli/src/`: the same flags, the
//! same sentences on failure, the same exit codes.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, Result, bail};
use compass_ipc::{Request, Response, SocketPath};

use crate::cli::{AppCommand, CmdCommand, FsCommand, ScriptCommand};
use crate::{EXIT_FAILURE, EXIT_OK, ipc, logs};

/// `compass theme template`: every key a theme file can set, the C++'s
/// `extra/theme-template.toml`.
pub const THEME_TEMPLATE: &str = include_str!("../../../extra/theme-template.toml");

/// The most `fs query --limit` asks the index for, as the C++ help says.
pub const MAX_FS_LIMIT: u32 = 10_000;

/// Set in the environment of an engine `compass server --no-extension-runtime`
/// starts: extensions are refused rather than run.
pub const NO_EXTENSION_RUNTIME_ENV: &str = "COMPASS_NO_EXTENSION_RUNTIME";

/// `compass version`: the C++'s three lines.
#[must_use]
pub fn version() -> String {
    let profile = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    format!(
        "Version {} (commit {})\nBuild: rustc - {profile} - {}/{}\nProvenance: {}\n",
        env!("CARGO_PKG_VERSION"),
        option_env!("COMPASS_GIT_COMMIT").unwrap_or("unknown"),
        std::env::consts::OS,
        std::env::consts::ARCH,
        option_env!("COMPASS_PROVENANCE").unwrap_or("local"),
    )
}

/// `compass server`: refuses while an engine answers, unless `replace`,
/// which kills it; then becomes the engine (`serve`), or the engine and its
/// window (`start`) with `open`.
///
/// # Errors
///
/// When an engine is running and `replace` was not passed, it cannot be
/// killed, or this binary cannot be executed.
pub async fn server(
    socket: &SocketPath,
    open: bool,
    replace: bool,
    config: Option<PathBuf>,
    no_extension_runtime: bool,
) -> Result<ExitCode> {
    if let Ok(Response::Pong { pid, .. }) = ipc::send(socket, Request::Ping).await {
        if !replace {
            bail!("A server is already running (pid {pid}). Pass --replace to replace it.");
        }
        eprintln!("Killing existing Compass server (pid {pid})...");
        let pid = i32::try_from(pid).context("the engine's pid is out of range")?;
        nix::sys::signal::kill(
            nix::unistd::Pid::from_raw(pid),
            nix::sys::signal::Signal::SIGKILL,
        )
        .map_err(|error| anyhow::anyhow!("Failed to kill process: {error}"))?;
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }

    let mut command = std::process::Command::new(std::env::current_exe()?);
    command
        .arg("--socket")
        .arg(socket.as_path())
        .arg(if open { "start" } else { "serve" });
    if let Some(config) = config {
        command.env(compass_core::config::CONFIG_PATH_ENV, config);
    }
    if no_extension_runtime {
        command.env(NO_EXTENSION_RUNTIME_ENV, "1");
    }
    let error = std::os::unix::process::CommandExt::exec(&mut command);
    bail!("Failed to exec server: {error}")
}

/// `compass cmd ls` and `compass cmd launch`.
///
/// # Errors
///
/// When the engine cannot be reached or refuses.
pub async fn cmd(socket: &SocketPath, command: CmdCommand) -> Result<ExitCode> {
    match command {
        CmdCommand::Ls { json } => {
            let commands = match ipc::send(socket, Request::ListCommands)
                .await
                .context("Failed to list commands")?
            {
                Response::Commands { commands } => commands,
                other => bail!("unexpected answer from the engine: {other:?}"),
            };
            if json {
                println!("{}", serde_json::to_string_pretty(&commands)?);
            } else {
                for command in &commands {
                    println!("{}", command.id);
                }
            }
        }
        CmdCommand::Launch {
            entrypoint,
            args,
            cwd,
            query,
        } => {
            let cwd = cwd.or_else(|| {
                std::env::current_dir()
                    .ok()
                    .map(|dir| dir.to_string_lossy().into_owned())
            });
            ipc::send_ack(
                socket,
                Request::LaunchCommand {
                    id: entrypoint,
                    args,
                    cwd,
                    query,
                },
            )
            .await
            .context("Failed to launch command")?;
        }
    }
    Ok(ExitCode::from(EXIT_OK))
}

/// `compass app launch`.
///
/// # Errors
///
/// When the engine cannot be reached, knows no such application, or the
/// launch fails.
pub async fn app(socket: &SocketPath, command: AppCommand) -> Result<ExitCode> {
    let AppCommand::Launch { app_id, args, new } = command;
    let request = Request::LaunchApp {
        id: app_id,
        args,
        new_instance: new,
    };
    match ipc::send(socket, request)
        .await
        .context("Failed to launch app")?
    {
        Response::AppLaunched {
            focused_window_title: Some(title),
        } => {
            println!(
                "Focused existing window \"{title}\"\nPass --new if you want to launch a new instance."
            );
        }
        Response::AppLaunched { .. } => {}
        other => bail!("unexpected answer from the engine: {other:?}"),
    }
    Ok(ExitCode::from(EXIT_OK))
}

/// `compass fs query`.
///
/// # Errors
///
/// A query under three characters, an engine that cannot be reached, or an
/// index that is not running.
pub async fn fs(socket: &SocketPath, command: FsCommand) -> Result<ExitCode> {
    let FsCommand::Query {
        query,
        limit,
        category,
        json,
    } = command;
    if query.chars().count() < 3 {
        bail!("Query should be at least 3 characters long");
    }
    let category = category
        .map(|name| {
            compass_core::file_search::filter_key_for_cli_name(&name)
                .map(str::to_owned)
                .with_context(|| format!("Unknown file category: {name}"))
        })
        .transpose()?;
    let request = Request::FsQuery {
        query,
        limit: limit.min(MAX_FS_LIMIT),
        category,
    };
    let files = match ipc::send(socket, request).await? {
        Response::Files { files, .. } => files,
        other => bail!("unexpected answer from the engine: {other:?}"),
    };
    if json {
        let rows: Vec<serde_json::Value> = files
            .iter()
            .map(|file| {
                serde_json::json!({
                    "path": file.path,
                    "category": compass_core::file_search::cli_name_for_filter_key(&file.category)
                        .unwrap_or("other"),
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&rows)?);
    } else {
        for file in &files {
            println!("{}", file.path);
        }
    }
    Ok(ExitCode::from(EXIT_OK))
}

/// `compass script template` and `compass script check`.
///
/// # Errors
///
/// An unknown language or mode for `template`.
pub fn script(command: ScriptCommand) -> Result<ExitCode> {
    use compass_core::script_command::{OutputMode, ScriptCommand as Parsed};
    use compass_core::script_template::{Language, MODES, generate};
    match command {
        ScriptCommand::Template { title, lang, mode } => {
            let Some(language) = Language::parse(&lang) else {
                bail!(
                    "Invalid language: {lang}\n\nSupported languages: {}",
                    Language::supported()
                );
            };
            let Some(mode) = OutputMode::parse(&mode) else {
                bail!("Invalid output mode: {mode}\n\nSupported modes: {MODES}");
            };
            println!("{}", generate(&title, language, mode));
            Ok(ExitCode::from(EXIT_OK))
        }
        ScriptCommand::Check { file } => {
            if !file.exists() {
                eprintln!("Error: File not found: {}", file.display());
                return Ok(ExitCode::from(EXIT_FAILURE));
            }
            match Parsed::from_file(&file) {
                Ok(_) => Ok(ExitCode::from(EXIT_OK)),
                Err(error) => {
                    eprintln!("Error: {error}");
                    Ok(ExitCode::from(EXIT_FAILURE))
                }
            }
        }
    }
}

/// `compass state open`: exits 0 when the window is open, 1 when not.
///
/// # Errors
///
/// When the engine cannot be reached.
pub async fn state_open(socket: &SocketPath) -> Result<ExitCode> {
    match ipc::send(socket, Request::DescribeWindow)
        .await
        .context("Failed to query open state")?
    {
        Response::WindowState { open } => {
            Ok(ExitCode::from(if open { EXIT_OK } else { EXIT_FAILURE }))
        }
        other => bail!("unexpected answer from the engine: {other:?}"),
    }
}

/// `compass logs`: the last `lines` lines of the engine's log, then, with
/// `follow`, what it writes next.
///
/// # Errors
///
/// When the log cannot be read or standard output written.
pub fn logs(lines: usize, follow: bool) -> Result<ExitCode> {
    let path = logs::log_path().context("no state directory to find the log in")?;
    let mut out = std::io::stdout().lock();
    let offset = logs::tail(&path, lines, &mut out)?;
    if offset.is_none() && !follow {
        eprintln!(
            "No log file at {}. Has the server been started yet?",
            path.display()
        );
        return Ok(ExitCode::from(EXIT_FAILURE));
    }
    if follow {
        logs::follow(&path, offset, &mut out, || true)?;
    }
    Ok(ExitCode::from(EXIT_OK))
}

/// `compass theme check`: whether the theme file parses, and what was
/// wrong with it that did not stop it.
///
/// # Errors
///
/// When the file cannot be read or is not a theme.
pub fn theme_check(file: &Path) -> Result<ExitCode> {
    let text =
        std::fs::read_to_string(file).with_context(|| format!("reading {}", file.display()))?;
    let theme = compass_core::theme_file::parse(file, &text)
        .map_err(|error| anyhow::anyhow!("Theme is invalid: {error}"))?;
    for diagnostic in &theme.diagnostics {
        println!("Warning: {diagnostic}");
    }
    println!("Theme file is valid");
    Ok(ExitCode::from(EXIT_OK))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_theme_template_is_a_valid_theme_with_nothing_to_warn_about() {
        let theme =
            compass_core::theme_file::parse(Path::new("template.toml"), THEME_TEMPLATE).unwrap();
        assert_eq!(theme.name, "Vicinae Dark");
        assert!(theme.dark);
        assert!(theme.diagnostics.is_empty(), "{:?}", theme.diagnostics);
    }

    #[test]
    fn the_version_is_the_cpps_three_lines() {
        let text = version();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 3);
        assert!(lines[0].starts_with(&format!("Version {} (commit ", env!("CARGO_PKG_VERSION"))));
        assert!(lines[1].starts_with("Build: "));
        assert!(lines[2].starts_with("Provenance: "));
    }
}
