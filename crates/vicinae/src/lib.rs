//! The `vicinae` command-line front end.
//!
//! Phase 2 of `docs/rust-engine/PLAN.md` §6: an IPC client for controlling a
//! running engine ([`ipc`]), and the diagnostic that tells a user — and a bug
//! report — what this machine can and cannot do ([`doctor`]).
//!
//! Everything is exposed as a library rather than buried in `main.rs` so the
//! checks, the report rendering and the command dispatch are all reachable from
//! tests. `main` is four lines.

#![warn(missing_docs)]
#![warn(missing_debug_implementations)]

pub mod appearance;
pub mod catalog_watch;
pub mod cli;
pub mod cli_commands;
pub mod clipboard_service;
pub mod config_cmd;
pub mod conformance;
pub mod developer;
pub mod dmenu;
pub mod doctor;
pub mod engine;
pub mod extension_apps;
pub mod extension_browser;
pub mod extension_commands;
pub mod extension_files;
pub mod extension_runner;
pub mod extension_wallpaper;
pub mod extension_windows;
pub mod file_manager;
pub mod file_search;
pub mod fonts;
pub mod hotkey;
pub mod indexer_client;
pub mod indexer_service;
pub mod indexer_watch;
pub mod input_server;
pub mod ipc;
pub mod logs;
pub mod notification_icon;
pub mod programs;
pub mod rhai_host;
pub mod rhai_scripts;
pub mod scripts;
pub mod serve;
pub mod session;
pub mod shortcuts;
pub mod snippet_expansion;
pub mod snippets;
pub mod spike;
pub mod stores;
pub mod tray_host;
pub mod typography;
pub mod ui_backend;
mod ui_instance;
pub mod vicinae_import;
pub mod window;
pub mod window_service;
pub mod wlroots;

use std::process::ExitCode;

use anyhow::{Context, Result, bail};
use clap::Parser;
use compass_ipc::{Request, Response};

pub use cli::{Cli, Command, ExtCommand, Spike};
pub use engine::Engine;

/// Exit code when the command did what it was asked.
pub const EXIT_OK: u8 = 0;
/// Exit code when the command failed, or `doctor --check-only` found a failure.
pub const EXIT_FAILURE: u8 = 1;

/// Parses the command line, runs the command, and reports failures.
///
/// Usage errors exit 2 from inside `clap`; see [`cli::EXIT_CODE_HELP`].
#[must_use]
pub fn main() -> ExitCode {
    let cli = Cli::parse_from(cli::with_deeplink(std::env::args_os().collect()));
    // The engine also writes its log to a file, for `vicinae logs`.
    let log_file = matches!(cli.command, Command::Serve { .. })
        .then(logs::log_path)
        .flatten()
        .map(|path| {
            let log = logs::LogFile::pending(&path);
            log.register();
            log
        });
    init_tracing(cli.verbose, log_file);

    match run(cli) {
        Ok(code) => code,
        Err(err) => {
            eprintln!("error: {err:#}");
            ExitCode::from(EXIT_FAILURE)
        }
    }
}

/// Runs an already-parsed command line on a fresh runtime.
///
/// The client commands get a current-thread runtime: they open one connection,
/// send one frame and exit, so a thread pool is pure startup cost on something
/// a person has bound to a key. `serve` gets a multi-threaded one, because
/// ranking a query is CPU work and on a single thread one slow query would
/// stall every other connection.
pub fn run(cli: Cli) -> Result<ExitCode> {
    // As early as a process can see of itself. Everything before this --
    // dynamic linking, which is not free for a binary that links wgpu -- is
    // outside it, which is why the figure derived from it is documented as a
    // floor. See `Message::FrameDrawn`.
    let started_at = std::time::Instant::now();
    // Before any runtime exists, and deliberately. Iced owns the thread it is
    // started on, and on Wayland that has to be the process's main thread —
    // so the launcher cannot be dispatched from inside `block_on` like every
    // other command. ADR-0011 records what this costs and what it defers.
    if matches!(cli.command, Command::Ui | Command::Start { .. }) {
        require_servable_engine(cli.engine)?;
        // Checked here rather than left to Iced. With no display, `iced::run`
        // does not return an error — winit panics inside it, and the user gets
        // a backtrace naming winit's source file for the entirely ordinary
        // situation of running the launcher from a TTY or over ssh. `doctor`
        // already diagnoses this properly, so point at it.
        if std::env::var_os("WAYLAND_DISPLAY").is_none()
            && std::env::var_os("WAYLAND_SOCKET").is_none()
            && std::env::var_os("DISPLAY").is_none()
        {
            bail!(
                "no graphical session: neither WAYLAND_DISPLAY nor DISPLAY is set, so there is \
                 nothing to open a window on. Run `vicinae doctor` for the full picture"
            );
        }
        let _ui_lease = match ui_instance::acquire(cli.socket_path().as_path())
            .context("claiming the resident launcher instance")?
        {
            Some(lease) => lease,
            None => {
                if matches!(cli.command, Command::Start { hidden: true }) {
                    return Ok(ExitCode::from(EXIT_OK));
                }
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()?;
                runtime.block_on(async {
                    tokio::time::timeout(
                        std::time::Duration::from_secs(5),
                        ipc::send_ack(&cli.socket_path(), Request::Show),
                    )
                    .await
                    .context("the existing launcher did not respond")?
                    .context("a launcher is already running but could not be shown")
                })?;
                return Ok(ExitCode::from(EXIT_OK));
            }
        };

        let _engine_session = if matches!(cli.command, Command::Start { .. }) {
            let mut command = std::process::Command::new(std::env::current_exe()?);
            command
                .arg("--engine=rust")
                .arg("--socket")
                .arg(cli.socket_path().as_path())
                .arg("serve")
                .stdin(std::process::Stdio::null());
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?;
            Some(runtime.block_on(session::ensure(
                &cli.socket_path(),
                &mut command,
                std::time::Duration::from_secs(15),
            ))?)
        } else {
            None
        };

        // Attached before Iced starts, on a thread that still belongs to us.
        // `None` means no engine is listening, which leaves the launcher
        // running undriven rather than refusing to start -- `vicinae ui` by
        // hand is a supported way to use it.
        let link = window::attach(cli.socket_path().as_path())
            .context("attaching the launcher window to the engine")?;
        if link.is_none() && matches!(cli.command, Command::Start { .. }) {
            bail!("the Compass engine stopped before the launcher could attach");
        }
        if link.is_none() {
            tracing::info!("no engine attached; Escape will exit rather than hide");
        }

        // The one place that knows which platform this is. ADR-0013: the
        // shared crates name what a platform can do; the binary picks who
        // does it.
        // The user's chord scheme. A configuration that cannot be read is not
        // a reason to refuse to start: the launcher runs with the defaults and
        // says so, which is what every other unreadable setting here does.
        let (
            keybinding,
            wrap_navigation,
            quick_launch,
            appearance_preset,
            color_scheme,
            theme_choice,
            root_config,
            configured_font,
            fallbacks,
            power_asks,
            browse_apps,
            emoji_skin_tone,
            clock,
        ) = match compass_core::Config::load() {
            Ok(config) => {
                let appearance = config.launcher().appearance();
                // A configured theme may be one of the user's theme files.
                let _ = compass_ui::theme::load_default_user_themes();
                (
                    config.launcher().keybinding_scheme(),
                    config.launcher().wrap_navigation(),
                    config.launcher().quick_launch(),
                    compass_ui::preset::resolve(
                        Some(appearance.preset()),
                        appearance.icons_override(),
                        appearance.tint_override(),
                    ),
                    appearance.color_scheme().to_owned(),
                    compass_ui::theme::Theme::from_name(appearance.theme()).unwrap_or_default(),
                    config.root_config(),
                    config.font_family().map(str::to_owned),
                    config.fallback_ids(),
                    power_asks(&config),
                    compass_core::browse_apps::Options::from_preferences(
                        config.entrypoint_preferences(
                            compass_core::commands::COMMANDS_PROVIDER_ID,
                            compass_core::browse_apps::ENTRYPOINT,
                        ),
                    ),
                    emoji_skin_tone(&config),
                    clock(&config),
                )
            }
            Err(error) => {
                tracing::warn!(%error, "could not read the configuration; using the defaults");
                (
                    compass_core::keybinding::Scheme::default(),
                    compass_core::config::DEFAULT_WRAP_NAVIGATION,
                    compass_core::config::DEFAULT_QUICK_LAUNCH,
                    compass_ui::preset::resolve(None, None, None),
                    compass_core::config::DEFAULT_COLOR_SCHEME.to_owned(),
                    compass_ui::theme::Theme::System,
                    compass_core::root_items::RootConfig::default(),
                    None,
                    compass_core::Config::default().fallback_ids(),
                    power_asks(&compass_core::Config::default()),
                    compass_core::browse_apps::Options::default(),
                    None,
                    clock(&compass_core::Config::default()),
                )
            }
        };

        // Said rather than swallowed: drawing the default for a name the user
        // typed leaves them adjusting a setting nothing is reading.
        if let Some(unknown) = &appearance_preset.unknown_name {
            let known: Vec<&str> = compass_ui::preset::NAMES
                .iter()
                .map(|(name, _)| *name)
                .collect();
            tracing::warn!(
                preset = %unknown,
                known = %known.join(", "),
                "unknown launcher.appearance.preset; using the default"
            );
        }

        let color_mode = appearance::ColorMode::from_config(&color_scheme);
        if !appearance::ColorMode::is_known(&color_scheme) {
            tracing::warn!(
                color_scheme = %color_scheme,
                "unknown launcher.appearance.color_scheme; following the system"
            );
        }

        // Read before the window opens so the first frame is the right
        // colour; see `appearance` for what happens when the portal is slow.
        let (appearance, appearance_link) = appearance::follow(color_mode);

        // The interface typeface: portal first, gsettings fallback, same
        // 250 ms budget as above so a wedged portal never blocks startup.
        // A family set in `font.normal.family` ("Set as vicinae font") wins
        // over the desktop's, which is then not followed.
        let (font_family, typography_link) = match configured_font {
            Some(family) => (Some(family), None),
            None => typography::follow(),
        };

        // One adapter serves both: application search and clipboard history
        // go to the same engine over the same socket.
        let daemon = link
            .as_ref()
            .map(|_| std::sync::Arc::new(ui_backend::DaemonBackend::new(cli.socket_path())));
        let backend = daemon
            .clone()
            .map(|d| d as std::sync::Arc<dyn compass_ui::backend::ApplicationBackend>);
        let clipboard = daemon
            .clone()
            .map(|d| d as std::sync::Arc<dyn compass_ui::backend::ClipboardBackend>);
        let windows = daemon.map(|d| d as std::sync::Arc<dyn compass_ui::backend::WindowBackend>);

        let flags = compass_ui::AppFlags {
            theme: theme_choice,
            launcher: std::sync::Arc::new(compass_platform_linux::LinuxLauncher),
            backend,
            clipboard,
            windows,
            root_config,
            link,
            exit_on_engine_disconnect: matches!(cli.command, Command::Start { .. }),
            start_hidden: matches!(cli.command, Command::Start { hidden: true }),
            keybinding,
            wrap_navigation,
            quick_launch,
            icons: appearance_preset.icons,
            appearance_preset,
            started_at: Some(started_at),
            appearance,
            appearance_link,
            font_family,
            typography_link,
            theme_dirs: compass_core::theme_file::default_search_dirs(),
            view_state_path: compass_ui::view_memory::default_path(),
            fallbacks,
            power_asks,
            browse_apps,
            glyph_path: compass_core::glyph_service::default_path(),
            builtin_icons: compass_core::builtin_icon::directory(),
            emoji_skin_tone,
            search_history_path: compass_core::root_view::default_history_path(),
            clock,
            ..compass_ui::AppFlags::default()
        };

        // The surface: a layer surface on the wlroots family, an
        // `xdg_toplevel` everywhere else, and always on GNOME.
        match launcher_surface() {
            compass_wayland::SurfaceKind::LayerShell => {
                tracing::info!("presenting the launcher as a wlr-layer-shell surface");
                compass_ui::run_resident_layer_shell(flags)
                    .map_err(|err| anyhow::anyhow!("the launcher could not start: {err}"))?;
            }
            compass_wayland::SurfaceKind::XdgToplevel => {
                compass_ui::run_resident(flags)
                    .map_err(|err| anyhow::anyhow!("the launcher could not start: {err}"))?;
            }
        }
        return Ok(ExitCode::from(EXIT_OK));
    }

    let runtime = if matches!(cli.command, Command::Serve { .. }) {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()?
    } else {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?
    };
    runtime.block_on(dispatch(cli))
}

/// Which surface the launcher window is, from the compositor's registry.
///
/// No compositor to ask (a probe that fails) is `xdg_toplevel`, which is what
/// Iced would have tried anyway, so its own error reaches the user unchanged.
/// Whether each power command asks first, from its `confirm` preference
/// (`providers.power.entrypoints.<id>.preferences`) or its own default.
fn power_asks(config: &compass_core::Config) -> std::collections::BTreeMap<String, bool> {
    use compass_core::power_commands::{COMMANDS, EXTENSION_ID, should_confirm};
    COMMANDS
        .iter()
        .map(|command| {
            let preferences = config.entrypoint_preferences(EXTENSION_ID, command.id);
            (command.id.to_owned(), should_confirm(command, preferences))
        })
        .collect()
}

/// The emoji picker's `skinTone` preference
/// (`providers.core.entrypoints.search-emojis.preferences.skinTone`, where the
/// C++ `SearchEmojiCommand` keeps it). `default` and an absent value are the
/// same: no modifier.
fn emoji_skin_tone(config: &compass_core::Config) -> Option<String> {
    config
        .entrypoint_preferences("core", "search-emojis")?
        .get("skinTone")?
        .as_str()
        .map(str::to_owned)
}

/// The root search's clock, from `launcher.clock`; `None` when it is off.
fn clock(config: &compass_core::Config) -> Option<compass_ui::ClockSettings> {
    let clock = config.launcher().clock();
    clock.enabled().then(|| compass_ui::ClockSettings {
        format: clock.format().to_owned(),
        interval: clock.interval(),
    })
}

fn launcher_surface() -> compass_wayland::SurfaceKind {
    match compass_wayland::Session::detect() {
        Ok(session) => compass_wayland::select_surface(
            &session,
            std::env::var(compass_wayland::layer_shell::OVERRIDE_ENV)
                .ok()
                .as_deref(),
        ),
        Err(err) => {
            tracing::debug!(error = %err, "no compositor to probe for a layer shell");
            compass_wayland::SurfaceKind::XdgToplevel
        }
    }
}

async fn dispatch(cli: Cli) -> Result<ExitCode> {
    let socket = cli.socket_path();

    match cli.command {
        Command::Doctor { check_only, json } => {
            // `doctor` runs whatever engine was asked for: refusing to
            // diagnose a machine because of the flag under diagnosis would be
            // perverse. The selection is reported as its own check instead.
            let report = doctor::run_on_this_machine(&socket, cli.engine).await;

            if json {
                println!("{}", serde_json::to_string_pretty(&report.to_json())?);
            } else {
                print!("{}", report.render_human(check_only));
            }

            Ok(ExitCode::from(doctor_exit_code(check_only, &report)))
        }

        Command::Ping => {
            require_servable_engine(cli.engine)?;
            println!("{}", ipc::ping(&socket).await?);
            Ok(ExitCode::from(EXIT_OK))
        }

        Command::Serve { no_hotkey } => {
            require_servable_engine(cli.engine)?;
            serve::run(&socket, !no_hotkey).await?;
            Ok(ExitCode::from(EXIT_OK))
        }

        Command::Shutdown => {
            require_servable_engine(cli.engine)?;
            match ipc::send(&socket, Request::Shutdown).await? {
                Response::ShuttingDown => Ok(ExitCode::from(EXIT_OK)),
                other => bail!("the engine answered {other:?} instead of shutting down"),
            }
        }

        Command::Query {
            text,
            json,
            provider,
        } => {
            require_servable_engine(cli.engine)?;
            let mut hits = ipc::query(&socket, &text.join(" ")).await?;
            if let Some(provider) = provider {
                let prefix = format!("{provider}:");
                hits.retain(|hit| hit.id.starts_with(&prefix));
            }

            if json {
                println!("{}", serde_json::to_string_pretty(&hits)?);
            } else {
                print!("{}", render_hits(&hits));
            }
            Ok(ExitCode::from(EXIT_OK))
        }

        Command::Ext(ExtCommand::List { json }) => {
            // Extension listing is a local operation; it doesn't need a running engine.
            // In the future this will query the engine for dynamically loaded extensions.
            let extensions = vec![serde_json::json!({
                "name": "gnome-shell",
                "status": "not_installed",
                "description": "GNOME Shell extension for window switching and clipboard"
            })];

            if json {
                println!("{}", serde_json::to_string_pretty(&extensions)?);
            } else {
                for ext in &extensions {
                    println!(
                        "{} - {} [{}]",
                        ext["name"], ext["description"], ext["status"]
                    );
                }
            }
            Ok(ExitCode::from(EXIT_OK))
        }

        Command::Spike(Spike::GlobalShortcut {
            trigger,
            id,
            wait,
            json,
        }) => {
            // Deliberately not gated on `require_servable_engine`: the spike
            // asks the *desktop* a question and never touches our engine, so
            // refusing to run it under --engine cpp would only make the answer
            // harder to get.
            let report = spike::shortcut::global_shortcut(&spike::shortcut::ShortcutSpike {
                id,
                description: "compass (spike): toggle the launcher".to_owned(),
                preferred_trigger: trigger,
                wait: std::time::Duration::from_secs(wait),
            })
            .await;

            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                print!("{}", report.render_human());
            }
            // Always zero. The report is the deliverable and every outcome in
            // it is a finding; a non-zero exit would make the harness treat
            // "GNOME said no" as a broken run.
            Ok(ExitCode::from(EXIT_OK))
        }

        Command::Deeplink { url } => {
            // The OAuth redirect and the store's extensions links; every other
            // deeplink the C++ takes (themes, commands) is refused by name
            // rather than silently dropped.
            if compass_worker_host::oauth_service::Redirect::parse(&url).is_ok() {
                ipc::send_ack(&socket, compass_ipc::Request::OAuthRedirect { url }).await?;
                return Ok(ExitCode::from(EXIT_OK));
            }
            match compass_core::store_listing::parse_extension_link(&url) {
                Some(Ok(_)) => {
                    ipc::send_ack(&socket, compass_ipc::Request::OpenDeeplink { url }).await?;
                    Ok(ExitCode::from(EXIT_OK))
                }
                Some(Err(usage)) => anyhow::bail!("{usage}"),
                None => anyhow::bail!("Compass does not handle this deeplink yet: {url}"),
            }
        }

        Command::Conformance {
            plan,
            timeout,
            json,
        } => {
            let report =
                conformance::run(plan.as_deref(), std::time::Duration::from_secs(timeout)).await?;
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                print!("{}", report.render_human());
            }
            Ok(ExitCode::from(if report.passed == report.total {
                EXIT_OK
            } else {
                EXIT_FAILURE
            }))
        }

        Command::Spike(Spike::Sandbox { json }) => {
            // Not gated on the engine either, and for a sharper reason than the
            // shortcut spike: this one confines the process it runs in, and
            // there is deliberately no way to undo that. It answers a question
            // about the machine, then exits.
            let report = spike::sandbox::run();
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                print!("{}", report.render_human());
            }
            Ok(ExitCode::from(EXIT_OK))
        }

        Command::Theme(theme_cmd) => handle_theme(theme_cmd).await,

        Command::Version => {
            print!("{}", cli_commands::version());
            Ok(ExitCode::from(EXIT_OK))
        }
        Command::Server {
            open,
            replace,
            config,
            no_extension_runtime,
        } => {
            require_servable_engine(cli.engine)?;
            cli_commands::server(&socket, open, replace, config, no_extension_runtime).await
        }
        Command::Cmd(command) => {
            require_servable_engine(cli.engine)?;
            cli_commands::cmd(&socket, command).await
        }
        Command::App(command) => {
            require_servable_engine(cli.engine)?;
            cli_commands::app(&socket, command).await
        }
        Command::Fs(command) => {
            require_servable_engine(cli.engine)?;
            cli_commands::fs(&socket, command).await
        }
        Command::Script(command) => cli_commands::script(command),
        Command::State(cli::StateCommand::Open) => {
            require_servable_engine(cli.engine)?;
            cli_commands::state_open(&socket).await
        }
        Command::Logs { lines, follow } => cli_commands::logs(lines, follow),

        Command::InputServer(command) => {
            require_servable_engine(cli.engine)?;
            handle_input_server(&socket, command).await
        }

        Command::Config(config_cmd) => config_cmd::run(config_cmd),

        // Handled in `run`, before the runtime exists.
        Command::Ui | Command::Start { .. } => {
            unreachable!("the launcher is dispatched before the runtime")
        }

        Command::Dmenu(args) => {
            require_servable_engine(cli.engine)?;
            let content = std::io::read_to_string(std::io::stdin())
                .context("reading the dmenu entries from standard input")?;
            let spec = crate::dmenu::spec(args, content);
            match ipc::send(&socket, Request::Dmenu { spec }).await? {
                Response::DmenuOutput { output } if output.is_empty() => {
                    Ok(ExitCode::from(EXIT_FAILURE))
                }
                Response::DmenuOutput { output } => {
                    println!("{output}");
                    Ok(ExitCode::from(EXIT_OK))
                }
                other => bail!("unexpected answer from the engine: {other:?}"),
            }
        }
        Command::Toggle => window_command(&socket, cli.engine, Request::Toggle).await,
        Command::Show => window_command(&socket, cli.engine, Request::Show).await,
        Command::Hide => window_command(&socket, cli.engine, Request::Hide).await,
    }
}

async fn handle_input_server(
    socket: &compass_ipc::SocketPath,
    command: crate::cli::InputServerCommand,
) -> Result<ExitCode> {
    use crate::cli::InputServerCommand;
    let (request, enable) = match command {
        InputServerCommand::Status { json } => {
            let status = match ipc::send(socket, Request::InputServerStatus).await? {
                Response::InputServerStatus(status) => status,
                other => bail!("unexpected answer from the engine: {other:?}"),
            };
            if json {
                println!("{}", serde_json::to_string_pretty(&status)?);
            } else {
                print!("{}", render_input_server(&status));
            }
            return Ok(ExitCode::from(if status.enabled && !status.running {
                EXIT_FAILURE
            } else {
                EXIT_OK
            }));
        }
        InputServerCommand::Enable => (Request::SetInputServerEnabled { enabled: true }, true),
        InputServerCommand::Disable => (Request::SetInputServerEnabled { enabled: false }, false),
    };
    match ipc::send(socket, request).await {
        Ok(Response::InputServerStatus(status)) => {
            print!("{}", render_input_server(&status));
            Ok(ExitCode::from(EXIT_OK))
        }
        Ok(other) => bail!("unexpected answer from the engine: {other:?}"),
        Err(error) => {
            // No engine: the setting still belongs in vicinae.json, and the
            // next engine reads it.
            tracing::debug!(%error, "no engine to apply input_server.enabled to");
            let mut config = compass_core::Config::load().unwrap_or_default();
            config.input_server_mut().set_enabled(Some(enable));
            config.save_to(compass_core::config::default_config_path()?)?;
            println!(
                "input server {}; the engine is not running, so it applies when it starts",
                if enable { "enabled" } else { "disabled" }
            );
            Ok(ExitCode::from(EXIT_OK))
        }
    }
}

/// `vicinae input-server status` for a person.
#[must_use]
pub fn render_input_server(status: &compass_ipc::InputServerStatus) -> String {
    let yes = |value: bool| if value { "yes" } else { "no" };
    let mut out = format!(
        "enabled:   {}\nrunning:   {}\ninjection: {}\nkeywords:  {}\nhelper:    {}\n",
        yes(status.enabled),
        yes(status.running),
        yes(status.injection),
        status.keywords,
        status.helper.as_deref().unwrap_or("not found"),
    );
    if let Some(problem) = &status.problem {
        out.push_str(&format!("problem:   {problem}\n"));
    }
    out
}

async fn handle_theme(cmd: crate::cli::ThemeCommand) -> Result<ExitCode> {
    use crate::cli::ThemeCommand;
    match cmd {
        ThemeCommand::List { json } => {
            let themes: Vec<_> = compass_ui::theme::Theme::ALL
                .iter()
                .copied()
                .chain(compass_ui::theme::load_default_user_themes())
                .map(|t| {
                    serde_json::json!({
                        "name": t.name(),
                        "description": t.description(),
                    })
                })
                .collect();
            if json {
                println!("{}", serde_json::to_string_pretty(&themes)?);
            } else {
                for t in &themes {
                    println!(
                        "{} - {}",
                        t["name"].as_str().unwrap(),
                        t["description"].as_str().unwrap()
                    );
                }
            }
            Ok(ExitCode::from(EXIT_OK))
        }
        ThemeCommand::Set { theme } => {
            let _ = compass_ui::theme::load_default_user_themes();
            let parsed = compass_ui::theme::Theme::from_name(&theme).ok_or_else(|| {
                anyhow::anyhow!("unknown theme {theme:?}; try `vicinae theme list`")
            })?;
            let mut config = compass_core::Config::load().unwrap_or_default();
            config
                .launcher_mut()
                .appearance_mut()
                .set_theme(Some(parsed.name().to_owned()));
            config.save_to(compass_core::config::default_config_path()?)?;
            println!("theme set to {}", parsed.name());
            Ok(ExitCode::from(EXIT_OK))
        }
        ThemeCommand::Reset => {
            let mut config = compass_core::Config::load().unwrap_or_default();
            config.launcher_mut().appearance_mut().set_theme(None);
            config.save_to(compass_core::config::default_config_path()?)?;
            println!("theme reset to system");
            Ok(ExitCode::from(EXIT_OK))
        }
        ThemeCommand::Template => {
            println!("{}", cli_commands::THEME_TEMPLATE);
            Ok(ExitCode::from(EXIT_OK))
        }
        ThemeCommand::Check { file } => cli_commands::theme_check(&file),
        ThemeCommand::Paths => {
            for dir in compass_core::theme_file::default_search_dirs() {
                println!("{}", dir.display());
            }
            Ok(ExitCode::from(EXIT_OK))
        }
    }
}

async fn window_command(
    socket: &compass_ipc::SocketPath,
    engine: Engine,
    request: Request,
) -> Result<ExitCode> {
    require_servable_engine(engine)?;
    ipc::send_ack(socket, request).await?;
    Ok(ExitCode::from(EXIT_OK))
}

/// Renders query hits for a terminal.
///
/// Deliberately plain and column-aligned rather than decorated: this output is
/// read by people debugging the index and piped into `grep` and `awk` at least
/// as often as it is read directly.
#[must_use]
pub fn render_hits(hits: &[compass_ipc::QueryHit]) -> String {
    if hits.is_empty() {
        return "no matches\n".to_owned();
    }

    let width = hits
        .iter()
        .map(|h| h.title.chars().count())
        .max()
        .unwrap_or(0);
    let mut out = String::new();
    for hit in hits {
        use std::fmt::Write as _;
        let pad = width - hit.title.chars().count();
        let _ = write!(out, "{:>3}  {}{:pad$}", hit.score, hit.title, "");
        match &hit.subtitle {
            Some(subtitle) => {
                let _ = writeln!(out, "  {subtitle}");
            }
            None => out.push('\n'),
        }
    }
    out
}

/// The exit code `doctor` reports.
///
/// Without `--check-only` the report is the product and the command succeeded
/// by producing it, whatever it says. With `--check-only` the exit code *is*
/// the product: non-zero for a failing check, zero for warnings, because
/// warnings are degradations this build is designed to run under (PLAN §3.5.1)
/// and failing CI on them would just get `--check-only` removed from the job.
#[must_use]
pub fn doctor_exit_code(check_only: bool, report: &doctor::Report) -> u8 {
    if check_only && report.has_failures() {
        EXIT_FAILURE
    } else {
        EXIT_OK
    }
}

/// Refuses commands that would have to be dispatched to an engine this binary
/// cannot start. See [`engine`] for why the default is `rust`.
fn require_servable_engine(engine: Engine) -> Result<()> {
    if engine.is_served_by_this_binary() {
        return Ok(());
    }
    anyhow::bail!(
        "--engine {engine} was selected, but this binary is the Rust engine and cannot dispatch \
         to the C++ one (PLAN.md §5: the dispatching front end arrives with the Phase 7 \
         cutover).\n\
         \x20 - run the C++ `vicinae` directly, or\n\
         \x20 - pass --engine rust / set COMPASS_ENGINE=rust\n\
         `vicinae doctor` still runs under either setting and reports which one is in effect."
    )
}

fn init_tracing(verbose: u8, log_file: Option<logs::LogFile>) {
    use tracing_subscriber::EnvFilter;
    use tracing_subscriber::Layer as _;
    use tracing_subscriber::layer::SubscriberExt as _;
    use tracing_subscriber::util::SubscriberInitExt as _;

    let default = match verbose {
        0 => "warn",
        1 => "info",
        2 => "debug",
        _ => "trace",
    };
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(format!("vicinae={default},compass_ipc={default}")));

    // A second call (from a test harness, say) is not an error worth aborting
    // for: the first subscriber wins.
    //
    // ANSI ONLY WHEN SOMETHING CAN RENDER IT. `tracing_subscriber::fmt` colours
    // unconditionally, so a redirected log gets escape sequences woven through
    // every field -- `applications\x1b[0m\x1b[2m=\x1b[0m15` rather than
    // `applications=15`. That is unreadable in a log file a user mails us, and
    // it silently broke a VM gate that grepped the engine's own output for a
    // count: the pattern matched nothing, so the gate reported the engine had
    // said nothing while printing the line where it had.
    let stderr = tracing_subscriber::fmt::layer()
        .with_writer(std::io::stderr)
        .with_ansi(std::io::IsTerminal::is_terminal(&std::io::stderr()));
    // The file gets at least `info`, as the C++ log file has everything but
    // debug output, whatever the terminal was asked for.
    let file = log_file.map(|file| {
        let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| {
            let level = if verbose > 1 { default } else { "info" };
            EnvFilter::new(format!("vicinae={level},compass_ipc={level}"))
        });
        tracing_subscriber::fmt::layer()
            .with_writer(file)
            .with_ansi(false)
            .with_filter(filter)
    });
    let _ = tracing_subscriber::registry()
        .with(stderr.with_filter(filter))
        .with(file)
        .try_init();
}

#[cfg(test)]
mod tests {
    use super::*;
    use compass_ipc::{DoctorCheck, DoctorStatus};

    fn report_of(statuses: &[DoctorStatus]) -> doctor::Report {
        doctor::Report {
            engine: Engine::Rust,
            socket: "/tmp/t.sock".to_string(),
            checks: statuses
                .iter()
                .enumerate()
                .map(|(i, status)| DoctorCheck {
                    name: format!("c{i}"),
                    status: *status,
                    detail: None,
                })
                .collect(),
        }
    }

    #[test]
    fn check_only_exits_zero_when_every_check_passes() {
        let report = report_of(&[DoctorStatus::Ok, DoctorStatus::Ok]);
        assert_eq!(doctor_exit_code(true, &report), EXIT_OK);
    }

    #[test]
    fn check_only_exits_zero_on_warnings_alone() {
        let report = report_of(&[DoctorStatus::Ok, DoctorStatus::Warn]);
        assert_eq!(doctor_exit_code(true, &report), EXIT_OK);
    }

    #[test]
    fn check_only_exits_non_zero_on_a_failure() {
        let report = report_of(&[DoctorStatus::Warn, DoctorStatus::Fail]);
        assert_eq!(doctor_exit_code(true, &report), EXIT_FAILURE);
        assert_ne!(EXIT_FAILURE, EXIT_OK);
    }

    #[test]
    fn without_check_only_a_failing_report_still_exits_zero() {
        let report = report_of(&[DoctorStatus::Fail]);
        assert_eq!(doctor_exit_code(false, &report), EXIT_OK);
    }

    #[test]
    fn the_rust_engine_is_servable_here() {
        assert!(require_servable_engine(Engine::Rust).is_ok());
    }

    #[test]
    fn the_cpp_engine_is_refused_with_an_explanation() {
        let err = require_servable_engine(Engine::Cpp).expect_err("cpp is not servable");
        let text = err.to_string();
        assert!(text.contains("cannot dispatch"));
        assert!(text.contains("--engine rust"));
        assert!(text.contains("doctor"));
    }
}
