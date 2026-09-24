//! The application database an extension reaches through `Application/*`.
//!
//! `open(url)`, `Action.OpenInBrowser` and `getApplications()` all land here.
//! The extension runs confined; the launch does not: it is the engine that
//! resolves the opener and spawns it, the same way the launcher would.

use std::sync::Arc;

use compass_platform::AppLauncher;
use compass_worker_host::application_service::{Application, Apps, TerminalOptions};
use compass_xdg::DesktopEntry;
use compass_xdg::mimeapps::{Lists, target_mime};

/// A snapshot of the installed applications and the MIME associations, taken
/// when the command starts.
#[derive(Debug)]
pub struct EngineApps {
    apps: Vec<(Application, Arc<DesktopEntry>)>,
    /// The ids of the applications in the `TerminalEmulator` category.
    terminals: Vec<String>,
    lists: Lists,
    /// The `xdg-terminals.list` files that exist, first wins.
    terminal_lists: Vec<std::path::PathBuf>,
    runtime: tokio::runtime::Handle,
}

impl EngineApps {
    /// The applications `index` knows, with the associations this session
    /// reads, launching on `runtime`.
    #[must_use]
    pub fn new(
        index: &compass_core::AppIndex,
        lists: Lists,
        runtime: tokio::runtime::Handle,
    ) -> Self {
        let terminals = index
            .launchable_items()
            .filter(|item| !item.is_action())
            .filter(|item| item.categories().iter().any(|c| c == "TerminalEmulator"))
            .map(|item| item.desktop_id().to_owned())
            .collect();
        let terminal_lists = compass_xdg::terminal::terminals_list_paths(
            compass_xdg::xdg_dirs::config_home().as_deref(),
            &compass_xdg::mimeapps::config_dirs(),
            &compass_xdg::xdg_dirs::data_dirs(),
            &compass_xdg::xdg_dirs::current_desktops(),
        )
        .into_iter()
        .filter(|path| path.is_file())
        .collect();
        let apps = index
            .launchable_items()
            .filter(|item| !item.is_action())
            .map(|item| {
                let app = Application {
                    id: item.desktop_id().to_owned(),
                    name: item.display_name(),
                    icon: item.icon().unwrap_or_default().to_owned(),
                    path: item
                        .path()
                        .map(|path| path.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                };
                (app, Arc::new(item.entry().clone()))
            })
            .collect();
        Self {
            apps,
            terminals,
            lists,
            terminal_lists,
            runtime,
        }
    }

    /// Opens `path` with the default application for its MIME type, as
    /// Search Files' `OpenFileAction` opens it with the first curated
    /// opener. `false` when no installed application opens that type.
    pub fn open_file(&self, path: &std::path::Path) -> bool {
        let path = path.to_string_lossy();
        let Some(opener) = self.default_opener(&path) else {
            return false;
        };
        self.launch(&opener, &path);
        true
    }

    /// The terminal to run a command in, as `XdgAppDatabase::terminalEmulator`
    /// chooses it: the first selected in the `xdg-terminals.list` files that
    /// is installed, else the first terminal they do not exclude, else what
    /// opens `x-scheme-handler/terminal`.
    fn terminal(&self) -> Option<&(Application, Arc<DesktopEntry>)> {
        use compass_xdg::terminal::{ListState, parse_terminals_list};
        let is_terminal = |id: &str| self.terminals.iter().any(|t| t == id);
        let mut seen = std::collections::BTreeSet::new();
        let mut selected = Vec::new();
        let mut excluded = std::collections::BTreeSet::new();
        for path in &self.terminal_lists {
            let Ok(text) = std::fs::read_to_string(path) else {
                continue;
            };
            for entry in parse_terminals_list(&text) {
                if !seen.insert(entry.id.clone()) {
                    continue;
                }
                match entry.state {
                    ListState::Selected => selected.push(entry.id),
                    ListState::Excluded => {
                        excluded.insert(entry.id);
                    }
                    ListState::Protected => {}
                }
            }
        }
        selected
            .iter()
            .find(|id| is_terminal(id))
            .or_else(|| self.terminals.iter().find(|id| !excluded.contains(*id)))
            .and_then(|id| self.find(id))
            .or_else(|| {
                let id = self
                    .lists
                    .default_for("x-scheme-handler/terminal", &self.usable())?;
                self.find(&id)
            })
    }

    /// The command line that runs `cmdline` in `terminal`: its own `Exec`,
    /// then the flags it declares (`X-TerminalArg*`) or is known to take.
    fn terminal_argv(
        terminal: &DesktopEntry,
        cmdline: &[String],
        options: &TerminalOptions,
    ) -> Option<Vec<String>> {
        use compass_xdg::terminal::{declared_args, terminal_args, terminal_command};
        let exec = terminal.expand_exec();
        let program = std::path::Path::new(exec.first()?)
            .file_name()?
            .to_string_lossy()
            .into_owned();
        let declared = terminal
            .path()
            .and_then(|path| std::fs::read_to_string(path).ok())
            .and_then(|text| {
                let reader = compass_xdg::reader::Reader::parse(
                    &text,
                    compass_xdg::locale::Locale::default(),
                );
                let group = reader.group("Desktop Entry")?.clone();
                declared_args(|key| group.raw(key).map(str::to_owned))
            });
        let args = terminal_args(declared, &program);
        let tail = terminal_command(
            "",
            &args,
            cmdline,
            options.title.as_deref(),
            options.working_directory.as_deref(),
            options.app_id.as_deref(),
            options.hold,
        );
        Some(exec.into_iter().chain(tail.into_iter().skip(1)).collect())
    }

    fn find(&self, id: &str) -> Option<&(Application, Arc<DesktopEntry>)> {
        self.apps.iter().find(|(app, _)| app.id == id)
    }

    fn usable(&self) -> impl Fn(&str) -> bool + '_ {
        |id| self.find(id).is_some()
    }

    /// The ids that can open `mime`: the lists' associations first, then
    /// every application whose own `MimeType=` claims it.
    fn opener_ids(&self, mime: &str) -> Vec<String> {
        let mut ids = self.lists.openers_for(mime, &self.usable());
        for (app, entry) in &self.apps {
            if entry.mime_types().iter().any(|claimed| claimed == mime) && !ids.contains(&app.id) {
                ids.push(app.id.clone());
            }
        }
        ids
    }
}

impl Apps for EngineApps {
    fn list(&self) -> Vec<Application> {
        self.apps.iter().map(|(app, _)| app.clone()).collect()
    }

    fn openers(&self, target: &str) -> Vec<Application> {
        self.opener_ids(&target_mime(target))
            .iter()
            .filter_map(|id| self.find(id).map(|(app, _)| app.clone()))
            .collect()
    }

    fn default_opener(&self, target: &str) -> Option<Application> {
        let mime = target_mime(target);
        self.lists
            .default_for(&mime, &self.usable())
            .or_else(|| self.opener_ids(&mime).into_iter().next())
            .and_then(|id| self.find(&id).map(|(app, _)| app.clone()))
    }

    fn by_id(&self, id: &str) -> Option<Application> {
        self.find(id)
            .or_else(|| self.find(&format!("{id}.desktop")))
            .map(|(app, _)| app.clone())
    }

    fn launch(&self, app: &Application, target: &str) {
        let Some((_, entry)) = self.find(&app.id) else {
            return;
        };
        let entry = Arc::clone(entry);
        let target = target.to_owned();
        let id = app.id.clone();
        self.runtime.spawn(async move {
            let uris: Vec<&str> = if target.is_empty() {
                Vec::new()
            } else {
                vec![target.as_str()]
            };
            if let Err(error) = compass_platform_linux::LinuxLauncher
                .launch(&entry, &uris)
                .await
            {
                tracing::warn!(%error, app = %id, "an extension's open did not launch");
            }
        });
    }

    fn show_in_file_browser(&self, target: &str, select: bool) {
        let path = std::path::Path::new(target);
        // Without FileManager1 the best a plain opener can do is the folder
        // the entry is in.
        let folder = if select || !path.is_dir() {
            path.parent().unwrap_or(path)
        } else {
            path
        };
        let folder = folder.to_string_lossy();
        if let Some(opener) = self
            .lists
            .default_for("inode/directory", &self.usable())
            .or_else(|| self.opener_ids("inode/directory").into_iter().next())
            .and_then(|id| self.find(&id).map(|(app, _)| app.clone()))
        {
            self.launch(&opener, &folder);
        }
    }

    fn run_in_terminal(&self, cmdline: &[String], options: &TerminalOptions) -> bool {
        if cmdline.is_empty() {
            return false;
        }
        // `options.app_id` is the new window's application id, passed to the
        // terminal's own flag; it does not choose the terminal.
        let Some((app, entry)) = self.terminal() else {
            tracing::warn!("an extension asked for a terminal, and none is installed");
            return false;
        };
        let Some(argv) = Self::terminal_argv(entry, cmdline, options) else {
            return false;
        };
        let id = app.id.clone();
        self.runtime.spawn(async move {
            if let Err(error) = compass_platform_linux::run_command(&argv).await {
                tracing::warn!(%error, terminal = %id, "an extension's terminal command did not start");
            }
        });
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apps(dir: &std::path::Path) -> EngineApps {
        std::fs::write(
            dir.join("browser.desktop"),
            "[Desktop Entry]\nType=Application\nName=Browser\nExec=browser %u\n\
             MimeType=x-scheme-handler/https;text/html;\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("files.desktop"),
            "[Desktop Entry]\nType=Application\nName=Files\nExec=files %U\n\
             MimeType=inode/directory;\n",
        )
        .unwrap();
        let index = compass_core::AppIndex::builder().dir(dir).build();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        EngineApps::new(&index, Lists::load(&[]), runtime.handle().clone())
    }

    #[test]
    fn a_command_runs_in_the_listed_terminal_with_the_flags_it_declares() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("xterm.desktop"),
            "[Desktop Entry]\nType=Application\nName=XTerm\nExec=xterm\n\
             Categories=System;TerminalEmulator;\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("org.gnome.Ptyxis.desktop"),
            "[Desktop Entry]\nType=Application\nName=Ptyxis\nExec=ptyxis --new-window\n\
             Categories=System;TerminalEmulator;\nX-TerminalArgExec=--\n\
             X-TerminalArgDir=--working-directory=\nX-TerminalArgTitle=--title\n",
        )
        .unwrap();
        let list = dir.path().join("xdg-terminals.list");
        std::fs::write(&list, "-xterm.desktop\n").unwrap();
        let index = compass_core::AppIndex::builder().dir(dir.path()).build();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let mut apps = EngineApps::new(&index, Lists::load(&[]), runtime.handle().clone());
        apps.terminal_lists = vec![list];

        let (terminal, entry) = apps.terminal().expect("a terminal");
        assert_eq!(terminal.id, "org.gnome.Ptyxis.desktop", "xterm is excluded");
        let argv = EngineApps::terminal_argv(
            entry,
            &["htop".to_owned(), "-d".to_owned(), "5".to_owned()],
            &TerminalOptions {
                hold: false,
                app_id: None,
                title: Some("Top".into()),
                working_directory: Some("/home/u".into()),
            },
        );
        assert_eq!(
            argv.unwrap(),
            [
                "ptyxis",
                "--new-window",
                "--title",
                "Top",
                "--working-directory=/home/u",
                "--",
                "htop",
                "-d",
                "5"
            ]
        );
    }

    #[test]
    fn a_link_opens_in_what_claims_its_scheme_and_ids_resolve_with_or_without_the_suffix() {
        let dir = tempfile::tempdir().unwrap();
        let apps = apps(dir.path());
        assert_eq!(
            apps.default_opener("https://example.com").map(|a| a.id),
            Some("browser.desktop".to_owned())
        );
        assert_eq!(
            apps.openers(&dir.path().to_string_lossy())
                .iter()
                .map(|a| a.id.as_str())
                .collect::<Vec<_>>(),
            ["files.desktop"],
            "a directory path is classified as inode/directory"
        );
        let page = dir.path().join("index.html");
        std::fs::write(&page, "<p>").unwrap();
        assert_eq!(
            apps.default_opener(&page.to_string_lossy()).map(|a| a.id),
            Some("browser.desktop".to_owned()),
            "a file path opens with what claims its MIME type, not the path as a type"
        );
        assert_eq!(apps.list().len(), 2);
        assert!(apps.by_id("files").is_some());
        assert!(apps.by_id("files.desktop").is_some());
        assert!(apps.default_opener("mailto:a@b").is_none());
    }
}
