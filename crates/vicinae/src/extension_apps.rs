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

/// Launches `entry` with `target` (none when empty).
async fn launch_entry(id: String, entry: Arc<DesktopEntry>, target: String) {
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
}

/// A local path as a `file://` URI, percent-encoded as `QUrl::fromLocalFile`.
fn file_uri(path: &std::path::Path) -> String {
    url::Url::from_file_path(path).map_or_else(
        |()| format!("file://{}", path.to_string_lossy()),
        |url| url.to_string(),
    )
}

/// The types `setWebBrowser` makes the browser the default for.
pub const WEB_BROWSER_MIMES: [&str; 4] = [
    "x-scheme-handler/http",
    "x-scheme-handler/https",
    "text/html",
    "application/xhtml+xml",
];

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

    /// Every application with what `AppService::findByClass` matches a
    /// window's class against, for the extension's window manager.
    #[must_use]
    pub fn window_classes(&self) -> Vec<(compass_core::app_windows::AppIdentity, Application)> {
        self.apps
            .iter()
            .map(|(app, entry)| {
                (
                    compass_core::app_windows::AppIdentity {
                        desktop_id: app.id.clone(),
                        startup_wm_class: entry.startup_wm_class().map(str::to_owned),
                        display_name: app.name.clone(),
                    },
                    app.clone(),
                )
            })
            .collect()
    }

    /// `XdgAppDatabase::webBrowser`: what opens `https`, then `http`, then
    /// HTML, then the first `WebBrowser`, then the first application that
    /// claims either scheme.
    #[must_use]
    pub fn web_browser(&self) -> Option<Application> {
        self.lookup(
            &[
                "x-scheme-handler/https",
                "x-scheme-handler/http",
                "text/html",
            ],
            "WebBrowser",
            &["x-scheme-handler/https", "x-scheme-handler/http"],
        )
    }

    /// `XdgAppDatabase::fileBrowser`: what opens a directory, then the first
    /// `FileManager`, then the first application that claims directories.
    #[must_use]
    pub fn file_browser(&self) -> Option<Application> {
        self.lookup(&["inode/directory"], "FileManager", &["inode/directory"])
    }

    /// `XdgAppDatabase::genericTextEditor`: what opens plain text, then the
    /// first `TextEditor`, then the first application that claims plain
    /// text.
    #[must_use]
    pub fn text_editor(&self) -> Option<Application> {
        self.lookup(&["text/plain"], "TextEditor", &["text/plain"])
    }

    /// `XdgAppDatabase::terminalEmulator`, as [`Self::terminal_name`] finds
    /// it.
    #[must_use]
    pub fn terminal_emulator(&self) -> Option<Application> {
        self.terminal().map(|(app, _)| app.clone())
    }

    /// The installed terminal emulators (the `TerminalEmulator` category),
    /// in index order: what Set Default Terminal offers.
    #[must_use]
    pub fn terminal_emulators(&self) -> Vec<Application> {
        self.terminals
            .iter()
            .filter_map(|id| self.find(id).map(|(app, _)| app.clone()))
            .collect()
    }

    /// `XdgAppDatabase::setWebBrowser`: makes `id` the default for
    /// [`WEB_BROWSER_MIMES`] in the `mimeapps.list` at `path`, reads the
    /// associations again from `search_paths` and answers whether every one
    /// of them now resolves to `id` — so a desktop-specific list that
    /// overrides the user's is reported as a failure rather than a success
    /// nobody sees.
    pub fn set_web_browser(
        &mut self,
        id: &str,
        path: &std::path::Path,
        search_paths: &[std::path::PathBuf],
    ) -> bool {
        self.set_default_for(&WEB_BROWSER_MIMES, id, path, search_paths)
    }

    /// `setDefaultForMimes`, which [`Self::set_web_browser`] is.
    pub fn set_default_for(
        &mut self,
        mimes: &[&str],
        id: &str,
        path: &std::path::Path,
        search_paths: &[std::path::PathBuf],
    ) -> bool {
        if let Err(error) = compass_xdg::mimeapps_writer::set_default_application(path, mimes, id) {
            tracing::warn!(%error, path = %path.display(), "could not write mimeapps.list");
            return false;
        }
        self.lists = Lists::load(search_paths);
        mimes.iter().all(|mime| {
            self.default_for_mime(mime)
                .is_some_and(|(app, _)| app.id == id)
        })
    }

    /// `defaultForMime`: the first usable `Default Applications` entry, else
    /// the first opener.
    fn default_for_mime(&self, mime: &str) -> Option<&(Application, Arc<DesktopEntry>)> {
        self.lists
            .default_for(mime, &self.usable())
            .or_else(|| self.opener_ids(mime).into_iter().next())
            .and_then(|id| self.find(&id))
    }

    /// The shape the three `XdgAppDatabase` lookups share: the defaults for
    /// `mimes` in turn, then `findByCategory(category)`, then the first
    /// application whose own `MimeType=` claims one of `claimed`.
    fn lookup(&self, mimes: &[&str], category: &str, claimed: &[&str]) -> Option<Application> {
        mimes
            .iter()
            .find_map(|mime| self.default_for_mime(mime))
            .or_else(|| {
                self.apps
                    .iter()
                    .find(|(_, entry)| entry.categories().iter().any(|c| c == category))
            })
            .or_else(|| {
                self.apps.iter().find(|(_, entry)| {
                    claimed
                        .iter()
                        .any(|mime| entry.mime_types().iter().any(|m| m == mime))
                })
            })
            .map(|(app, _)| app.clone())
    }

    /// The name of the terminal a command would run in, if one is
    /// installed.
    #[must_use]
    pub fn terminal_name(&self) -> Option<String> {
        self.terminal().map(|(app, _)| app.name.clone())
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
        self.runtime.spawn(launch_entry(
            app.id.clone(),
            Arc::clone(entry),
            target.to_owned(),
        ));
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
        let folder = folder.to_string_lossy().into_owned();
        let opener = self
            .lists
            .default_for("inode/directory", &self.usable())
            .or_else(|| self.opener_ids("inode/directory").into_iter().next())
            .and_then(|id| {
                self.find(&id)
                    .map(|(app, entry)| (app.id.clone(), Arc::clone(entry)))
            });
        if !select {
            if let Some((id, entry)) = opener {
                self.runtime.spawn(launch_entry(id, entry, folder));
            }
            return;
        }
        // `FileManager1.ShowItems` opens the folder with the file selected,
        // which is what the C++'s `showInFileBrowser` asks; a desktop whose
        // file manager does not implement it gets the folder.
        let uri = file_uri(path);
        self.runtime.spawn(async move {
            match crate::file_manager::show_items(&uri).await {
                Ok(()) => {}
                Err(error) => {
                    tracing::info!(%error, "FileManager1 did not show the file; opening its folder");
                    if let Some((id, entry)) = opener {
                        launch_entry(id, entry, folder).await;
                    }
                }
            }
        });
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

    /// `webBrowser`, `fileBrowser` and `genericTextEditor`: the default for
    /// the type first, then the category, then whatever claims the type.
    #[test]
    fn the_browser_file_manager_and_editor_are_the_defaults_then_the_category_then_a_claim() {
        let dir = tempfile::tempdir().unwrap();
        let write = |file: &str, body: &str| {
            std::fs::write(
                dir.path().join(file),
                format!("[Desktop Entry]\nType=Application\nExec=x\n{body}"),
            )
            .unwrap();
        };
        write("aa-reader.desktop", "Name=Reader\nMimeType=text/plain;\n");
        write(
            "bb-notes.desktop",
            "Name=Notes\nCategories=Utility;TextEditor;\n",
        );
        write(
            "cc-web.desktop",
            "Name=Web\nMimeType=x-scheme-handler/http;\n",
        );
        write(
            "dd-surf.desktop",
            "Name=Surf\nCategories=Network;WebBrowser;\n",
        );
        write("ee-gedit.desktop", "Name=Gedit\nMimeType=text/plain;\n");
        let index = compass_core::AppIndex::builder().dir(dir.path()).build();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let lists = dir.path().join("mimeapps.list");
        std::fs::write(
            &lists,
            "[Default Applications]\ntext/plain=ee-gedit.desktop;\n",
        )
        .unwrap();
        let mut apps = EngineApps::new(
            &index,
            Lists::load(std::slice::from_ref(&lists)),
            runtime.handle().clone(),
        );
        let id = |app: Option<Application>| app.map(|app| app.id);

        assert_eq!(
            id(apps.text_editor()),
            Some("ee-gedit.desktop".into()),
            "the default"
        );
        assert_eq!(
            id(apps.web_browser()),
            Some("cc-web.desktop".into()),
            "what claims http is an opener, which comes before the category"
        );
        assert_eq!(id(apps.file_browser()), None, "nothing opens a directory");

        apps.lists = Lists::load(&[]);
        assert_eq!(
            id(apps.text_editor()),
            Some("aa-reader.desktop".into()),
            "without a default, the first opener"
        );

        let user = dir.path().join("config/mimeapps.list");
        assert!(apps.set_web_browser("dd-surf.desktop", &user, std::slice::from_ref(&user)));
        assert_eq!(id(apps.web_browser()), Some("dd-surf.desktop".into()));
        let written = std::fs::read_to_string(&user).unwrap();
        for mime in WEB_BROWSER_MIMES {
            assert!(
                written.contains(&format!("{mime}=dd-surf.desktop")),
                "{written}"
            );
        }

        let system = dir.path().join("system-mimeapps.list");
        std::fs::write(
            &system,
            "[Default Applications]\nx-scheme-handler/https=cc-web.desktop;\n",
        )
        .unwrap();
        assert!(
            !apps.set_web_browser("dd-surf.desktop", &user, &[system, user.clone()]),
            "a list read first that still says otherwise is a failure"
        );
    }
}
