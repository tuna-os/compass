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
    lists: Lists,
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
            lists,
            runtime,
        }
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

    fn run_in_terminal(&self, cmdline: &[String], _options: &TerminalOptions) -> bool {
        tracing::warn!(
            ?cmdline,
            "an extension asked to run a command in a terminal, which Compass does not do yet"
        );
        false
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
    fn a_link_opens_in_what_claims_its_scheme_and_ids_resolve_with_or_without_the_suffix() {
        let dir = tempfile::tempdir().unwrap();
        let apps = apps(dir.path());
        assert_eq!(
            apps.default_opener("https://example.com").map(|a| a.id),
            Some("browser.desktop".to_owned())
        );
        assert_eq!(
            apps.openers("/home")
                .iter()
                .map(|a| a.id.as_str())
                .collect::<Vec<_>>(),
            Vec::<&str>::new(),
            "a bare path is not classified yet, so nothing claims it"
        );
        assert_eq!(apps.list().len(), 2);
        assert!(apps.by_id("files").is_some());
        assert!(apps.by_id("files.desktop").is_some());
        assert!(apps.default_opener("mailto:a@b").is_none());
    }
}
