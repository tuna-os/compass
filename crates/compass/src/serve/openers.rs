//! "Open with…": the applications that handle a target, and opening it with
//! one of them (the C++ `OpenWithAction` over `AppService::findOpeners`,
//! and `OpenAppAction`).

use compass_ipc::{ErrorKind, OpenerEntry, ProtocolError, Response};
use compass_worker_host::application_service::Apps;

/// The applications that open `target`, the default one first and marked,
/// as `findOpeners` orders them (the default association, then the added
/// ones, then every application that claims the type).
#[must_use]
pub fn openers(apps: &impl Apps, target: &str) -> Vec<OpenerEntry> {
    let default = apps.default_opener(target);
    let mut found = apps.openers(target);
    if let Some(default) = &default {
        match found.iter().position(|app| app.id == default.id) {
            Some(position) => {
                let app = found.remove(position);
                found.insert(0, app);
            }
            None => found.insert(0, default.clone()),
        }
    }
    found
        .into_iter()
        .map(|app| OpenerEntry {
            default: default.as_ref().is_some_and(|d| d.id == app.id),
            icon: (!app.icon.is_empty()).then_some(app.icon),
            id: app.id,
            name: app.name,
        })
        .collect()
}

/// `OpenWith`: opens `target` with the application `id`.
pub fn open_with(apps: &impl Apps, id: &str, target: &str) -> Response {
    let Some(app) = apps.by_id(id) else {
        return Response::Error(ProtocolError::new(
            ErrorKind::BadRequest,
            format!("No application has the id {id}"),
        ));
    };
    apps.launch(&app, target);
    Response::Ack
}

#[cfg(test)]
mod tests {
    use super::*;
    use compass_worker_host::application_service::{Application, TerminalOptions};

    #[derive(Debug, Default)]
    struct FakeApps {
        apps: Vec<Application>,
        default: Option<&'static str>,
        launched: std::sync::Mutex<Vec<(String, String)>>,
    }

    fn app(id: &str, name: &str) -> Application {
        Application {
            id: id.into(),
            name: name.into(),
            icon: String::new(),
            path: String::new(),
        }
    }

    impl Apps for FakeApps {
        fn list(&self) -> Vec<Application> {
            self.apps.clone()
        }
        fn openers(&self, _target: &str) -> Vec<Application> {
            self.apps.clone()
        }
        fn default_opener(&self, _target: &str) -> Option<Application> {
            self.default.and_then(|id| self.by_id(id))
        }
        fn by_id(&self, id: &str) -> Option<Application> {
            self.apps.iter().find(|app| app.id == id).cloned()
        }
        fn launch(&self, app: &Application, target: &str) {
            self.launched
                .lock()
                .unwrap()
                .push((app.id.clone(), target.to_owned()));
        }
        fn show_in_file_browser(&self, _target: &str, _select: bool) {}
        fn run_in_terminal(&self, _cmdline: &[String], _options: &TerminalOptions) -> bool {
            false
        }
    }

    #[test]
    fn the_default_opener_comes_first_and_is_marked() {
        let apps = FakeApps {
            apps: vec![
                app("gimp.desktop", "GIMP"),
                app("loupe.desktop", "Image Viewer"),
            ],
            default: Some("loupe.desktop"),
            ..FakeApps::default()
        };
        let listed = openers(&apps, "/tmp/a.png");
        assert_eq!(
            listed
                .iter()
                .map(|o| (o.id.as_str(), o.default))
                .collect::<Vec<_>>(),
            [("loupe.desktop", true), ("gimp.desktop", false)]
        );
        let none = FakeApps {
            apps: vec![app("gimp.desktop", "GIMP")],
            ..FakeApps::default()
        };
        assert!(!openers(&none, "x")[0].default, "no default, none marked");
    }

    #[test]
    fn open_with_launches_the_chosen_application_and_refuses_an_unknown_one() {
        let apps = FakeApps {
            apps: vec![app("gimp.desktop", "GIMP")],
            ..FakeApps::default()
        };
        assert_eq!(
            open_with(&apps, "gimp.desktop", "/tmp/a.png"),
            Response::Ack
        );
        assert_eq!(
            apps.launched.lock().unwrap().as_slice(),
            [("gimp.desktop".to_owned(), "/tmp/a.png".to_owned())]
        );
        assert!(matches!(
            open_with(&apps, "nothing.desktop", "/tmp/a.png"),
            Response::Error(e) if e.kind == ErrorKind::BadRequest
        ));
    }
}
