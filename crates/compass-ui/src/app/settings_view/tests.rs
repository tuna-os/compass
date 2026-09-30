//! The settings view against a real launcher state: opening it, each kind of
//! control, the extension page's switches, aliases and shortcuts, and who
//! writes the file.

use std::sync::{Arc, Mutex};

use super::*;
use crate::app::AppFlags;
use crate::backend::BackendFuture;
use serde_json::json;

fn settle(app: &mut LauncherApp, task: Task<Message>) {
    use iced::futures::{StreamExt, executor::block_on};
    use iced_winit::runtime::{Action, task};
    let Some(stream) = task::into_stream(task) else {
        return;
    };
    for action in block_on(stream.collect::<Vec<_>>()) {
        if let Action::Output(message) = action {
            let next = app.update(message);
            settle(app, next);
        }
    }
}

fn key(key: Key, modifiers: Modifiers) -> Message {
    let physical =
        iced::keyboard::key::Physical::Unidentified(iced::keyboard::key::NativeCode::Unidentified);
    Message::Keyboard(iced::keyboard::Event::KeyPressed {
        key: key.clone(),
        modified_key: key,
        physical_key: physical,
        location: iced::keyboard::Location::Standard,
        modifiers,
        text: None,
        repeat: false,
    })
}

fn app(dir: &std::path::Path) -> LauncherApp {
    for (file, name) in [("firefox.desktop", "Firefox"), ("files.desktop", "Files")] {
        std::fs::write(
            dir.join(file),
            format!("[Desktop Entry]\nType=Application\nName={name}\nExec=/bin/true\n"),
        )
        .unwrap();
    }
    let mut app = LauncherApp::with_index(compass_core::AppIndex::builder().dir(dir).build());
    app.config_path = Some(dir.join("compass.json"));
    app
}

fn send(app: &mut LauncherApp, message: SettingsMessage) {
    let task = app.update(Message::Settings(message));
    settle(app, task);
}

fn saved(app: &LauncherApp) -> compass_core::Config {
    compass_core::Config::load_from(app.config_path.as_ref().unwrap()).unwrap()
}

fn page(app: &LauncherApp) -> &SettingsPage {
    let Page::Settings(page) = &app.page else {
        panic!("not the settings: {}", app.state_line());
    };
    page
}

fn select(app: &mut LauncherApp, key: &str) {
    let row = page(app).sidebar.index_of_key(key);
    assert!(row >= 0, "no sidebar row {key}");
    send(app, SettingsMessage::SidebarSelected(row as usize));
}

#[test]
fn open_settings_is_a_root_command_and_ctrl_comma_and_escape_leaves() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = app(dir.path());
    let _ = app.update(Message::QueryChanged("open settings".into()));
    let Some(crate::app::RootRow::Command(command)) = app.selected_row() else {
        panic!("no command: {}", app.state_line());
    };
    assert_eq!(command.id(), "commands:settings");
    let task = app.update(Message::LaunchSelected);
    settle(&mut app, task);
    assert_eq!(page(&app).shown(), Shown::Core(CorePage::General));

    let _ = app.update(key(Key::Named(Named::Escape), Modifiers::empty()));
    assert!(matches!(app.page, Page::Root));
    let _ = app.update(Message::QueryChanged(String::new()));
    let _ = app.update(key(Key::Character(",".into()), Modifiers::CTRL));
    assert_eq!(page(&app).shown(), Shown::Core(CorePage::General));

    // The arrows walk the sidebar, past the divider.
    for _ in 0..4 {
        let _ = app.update(key(Key::Named(Named::ArrowDown), Modifiers::empty()));
    }
    assert_eq!(page(&app).shown(), Shown::Core(CorePage::About));
    let _ = app.update(key(Key::Named(Named::ArrowDown), Modifiers::empty()));
    assert!(matches!(page(&app).shown(), Shown::Provider(_)));

    // The search field filters it.
    let _ = app.update(Message::Settings(SettingsMessage::QueryChanged(
        "keyb".into(),
    )));
    assert_eq!(page(&app).shown(), Shown::Core(CorePage::Keybindings));
}

#[test]
fn every_page_and_the_recorder_draw() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = app(dir.path());
    let _ = app.open_settings(None);
    let rows = page(&app).sidebar.rows().len();
    for row in 0..rows {
        send(&mut app, SettingsMessage::SidebarSelected(row));
        let _ = app.view();
    }
    send(
        &mut app,
        SettingsMessage::Record(RecordTarget::Setting("launcher.hotkey".into())),
    );
    let _ = app.view();
    send(&mut app, SettingsMessage::QueryChanged("zzzz".into()));
    let _ = app.view();
}

#[test]
fn a_root_rows_open_preferences_opens_the_settings_at_its_provider() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = app(dir.path());
    let _ = app.update(Message::QueryChanged("firefox".into()));
    let _ = app.update(Message::TogglePanel);
    let row = app
        .panel
        .as_ref()
        .and_then(|panel| panel.row_titled("Open Preferences"))
        .expect("the panel offers the preferences");
    let task = app.update(Message::PanelClicked(row));
    settle(&mut app, task);
    assert!(app.panel.is_none());
    assert!(matches!(page(&app).shown(), Shown::Provider(p) if p.id == "applications"));
}

#[test]
fn a_deeplink_opens_the_tab_it_names() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = app(dir.path());
    let _ = app.open_deeplink("vicinae://settings/open?tab=about");
    assert_eq!(page(&app).shown(), Shown::Core(CorePage::About));
    let _ = app.open_deeplink("vicinae://settings/open?tab=shortcuts");
    assert_eq!(page(&app).shown(), Shown::Core(CorePage::Keybindings));
    let _ = app.open_deeplink("vicinae://settings/open?tab=applications");
    assert!(matches!(page(&app).shown(), Shown::Provider(p) if p.id == "applications"));
}

#[test]
fn each_control_writes_the_file_and_the_launcher_follows_at_once() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = app(dir.path());
    std::fs::write(dir.path().join("compass.json"), r#"{"mystery": 1}"#).unwrap();
    let _ = app.open_settings(Some("advanced"));

    send(
        &mut app,
        SettingsMessage::Changed("launcher.wrap_navigation".into(), json!(true)),
    );
    send(
        &mut app,
        SettingsMessage::Changed("launcher.keybinding".into(), json!("emacs")),
    );
    assert!(app.wrap_navigation);
    assert_eq!(app.keybinding, compass_core::keybinding::Scheme::Emacs);
    let config = saved(&app);
    assert!(config.launcher().wrap_navigation());
    assert_eq!(config.launcher().keybinding(), "emacs");
    assert_eq!(
        config.get_path("mystery"),
        Some(json!(1)),
        "the rest is kept"
    );

    // A number is typed, and refused until it is one.
    select(&mut app, "general");
    send(
        &mut app,
        SettingsMessage::DraftEdited("launcher.max_results".into(), "lots".into()),
    );
    send(
        &mut app,
        SettingsMessage::DraftSubmitted("launcher.max_results".into()),
    );
    assert!(page(&app).notice.is_some());
    assert_eq!(saved(&app).launcher().max_results(), 50);
    send(
        &mut app,
        SettingsMessage::DraftEdited("launcher.max_results".into(), "12".into()),
    );
    send(
        &mut app,
        SettingsMessage::DraftSubmitted("launcher.max_results".into()),
    );
    assert_eq!(saved(&app).launcher().max_results(), 12);
    assert!(page(&app).notice.is_none());

    // The clock goes at once.
    send(
        &mut app,
        SettingsMessage::Changed("launcher.clock.enabled".into(), json!(false)),
    );
    assert!(app.clock.is_none());

    // A builtin command's preference, under its row.
    send(
        &mut app,
        SettingsMessage::Changed(
            "providers.power.entrypoints.lock.preferences.confirm".into(),
            json!(true),
        ),
    );
    assert_eq!(app.power_asks.get("lock"), Some(&true));
}

#[test]
fn the_extension_page_switches_aliases_and_records_shortcuts() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = app(dir.path());
    let _ = app.open_settings(Some("applications"));
    let firefox = page(&app)
        .providers
        .iter()
        .flat_map(|provider| &provider.items)
        .find(|item| item.title == "Firefox")
        .expect("Firefox is listed")
        .id
        .clone();
    let enabled = |app: &LauncherApp| app.app_index.root(&firefox).unwrap().meta.enabled;
    let (provider, entrypoint) = compass_core::root_items::split_entrypoint_id(&firefox).unwrap();

    send(
        &mut app,
        SettingsMessage::ItemToggled(firefox.clone(), false),
    );
    assert!(!enabled(&app));
    assert_eq!(
        saved(&app).root_config().providers[provider].entrypoints[entrypoint].enabled,
        Some(false)
    );
    send(
        &mut app,
        SettingsMessage::ItemToggled(firefox.clone(), true),
    );
    assert!(enabled(&app));

    send(
        &mut app,
        SettingsMessage::ProviderToggled("applications".into(), false),
    );
    assert!(!enabled(&app), "the provider's switch covers its items");
    assert_eq!(
        saved(&app).root_config().providers["applications"].enabled,
        Some(false)
    );
    let row = page(&app)
        .sidebar
        .rows()
        .iter()
        .find(|row| row.key == "applications")
        .unwrap();
    assert!(!row.enabled, "the sidebar greys it");
    send(
        &mut app,
        SettingsMessage::ProviderToggled("applications".into(), true),
    );
    assert!(enabled(&app));

    send(
        &mut app,
        SettingsMessage::DraftEdited(alias_key(&firefox), "ff".into()),
    );
    send(
        &mut app,
        SettingsMessage::DraftSubmitted(alias_key(&firefox)),
    );
    assert_eq!(
        app.app_index.root(&firefox).unwrap().meta.alias.as_deref(),
        Some("ff")
    );
    assert_eq!(
        saved(&app).root_config().providers[provider].entrypoints[entrypoint]
            .alias
            .as_deref(),
        Some("ff")
    );

    send(
        &mut app,
        SettingsMessage::Record(RecordTarget::Item(firefox.clone())),
    );
    assert!(page(&app).recorder.is_some());
    let _ = app.update(key(Key::Named(Named::Control), Modifiers::CTRL));
    let task = app.update(key(Key::Character("j".into()), Modifiers::CTRL));
    settle(&mut app, task);
    assert!(
        page(&app).recorder.is_none(),
        "an accepted shortcut ends it"
    );
    assert_eq!(
        app.app_index
            .root(&firefox)
            .unwrap()
            .meta
            .shortcut
            .as_deref(),
        Some("control+J")
    );
    assert_eq!(
        saved(&app).root_config().providers[provider].entrypoints[entrypoint]
            .shortcut
            .as_deref(),
        Some("control+J")
    );
}

#[test]
fn the_hotkey_is_recorded_into_the_launcher_section() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = app(dir.path());
    let _ = app.open_settings(None);
    send(
        &mut app,
        SettingsMessage::Record(RecordTarget::Setting("launcher.hotkey".into())),
    );
    assert!(
        app.shortcuts_inhibited(),
        "the compositor's shortcuts reach the recorder"
    );
    let _ = app.update(key(Key::Named(Named::Alt), Modifiers::ALT));
    let task = app.update(key(Key::Named(Named::Space), Modifiers::ALT));
    settle(&mut app, task);
    assert!(page(&app).recorder.is_none());
    assert!(!app.shortcuts_inhibited(), "and are given back after");
    let hotkey = saved(&app).launcher().hotkey().to_owned();
    assert!(
        compass_core::key_combo::KeyCombo::parse(&hotkey).is_some(),
        "{hotkey}"
    );
    assert_ne!(hotkey, compass_core::config::DEFAULT_HOTKEY);
}

#[derive(Debug, Default)]
struct Engine {
    settings: Mutex<Vec<(String, serde_json::Value)>>,
    providers: Mutex<Vec<(String, bool)>>,
    refuse: bool,
}

impl crate::backend::ApplicationBackend for Engine {
    fn search(&self, _query: String) -> BackendFuture<'_, Vec<String>> {
        Box::pin(async { Ok(Vec::new()) })
    }
    fn record_launch(&self, _key: String) -> BackendFuture<'_, ()> {
        Box::pin(async { Ok(()) })
    }
    fn edit_root_item(&self, _id: String, _edit: RootEdit) -> BackendFuture<'_, ()> {
        Box::pin(async { Ok(()) })
    }
    fn set_setting(&self, key: String, value: serde_json::Value) -> BackendFuture<'_, ()> {
        self.settings.lock().unwrap().push((key, value));
        let refuse = self.refuse;
        Box::pin(async move {
            if refuse {
                Err("unknown theme".to_owned())
            } else {
                Ok(())
            }
        })
    }
    fn set_provider_enabled(&self, provider: String, enabled: bool) -> BackendFuture<'_, ()> {
        self.providers.lock().unwrap().push((provider, enabled));
        Box::pin(async { Ok(()) })
    }
    fn extension_preferences(
        &self,
        id: String,
    ) -> BackendFuture<'_, crate::backend::ExtensionStart> {
        let refuse = self.refuse;
        Box::pin(async move {
            if refuse {
                return Err("no keyring".to_owned());
            }
            Ok(crate::backend::ExtensionStart::NeedsPreferences {
                title: id,
                fields: Vec::new(),
            })
        })
    }
}

#[test]
fn a_commands_preferences_open_over_the_settings_and_go_back_to_them() {
    for refuse in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app(dir.path());
        app.apply(AppFlags {
            backend: Some(Arc::new(Engine {
                refuse,
                ..Engine::default()
            })),
            ..AppFlags::default()
        });
        let _ = app.open_settings(Some("applications"));
        send(
            &mut app,
            SettingsMessage::OpenPreferences("@me/notes:list".into()),
        );
        if refuse {
            assert_eq!(page(&app).notice.as_deref(), Some("no keyring"));
            continue;
        }
        assert!(
            matches!(&app.page, Page::Preferences(form)
                if form.purpose == crate::preferences_page::Purpose::CommandPreferences),
            "{}",
            app.state_line()
        );
        let task = app.update(Message::Back);
        settle(&mut app, task);
        assert!(
            matches!(page(&app).shown(), Shown::Provider(p) if p.id == "applications"),
            "back where it was"
        );
    }
}

#[test]
fn with_an_engine_the_engine_writes_and_a_theme_is_kept_or_put_back() {
    for refuse in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let engine = Arc::new(Engine {
            refuse,
            ..Engine::default()
        });
        let mut app = app(dir.path());
        app.apply(AppFlags {
            backend: Some(engine.clone()),
            ..AppFlags::default()
        });
        app.config_path = Some(dir.path().join("compass.json"));
        let _ = app.open_settings(Some("appearance"));
        send(
            &mut app,
            SettingsMessage::Changed("launcher.appearance.theme".into(), json!("nord")),
        );
        assert_eq!(
            engine.settings.lock().unwrap().as_slice(),
            &[("launcher.appearance.theme".to_owned(), json!("nord"))]
        );
        if refuse {
            assert_eq!(app.theme_choice, crate::theme::Theme::System, "put back");
            assert_eq!(page(&app).notice.as_deref(), Some("unknown theme"));
        } else {
            assert_eq!(app.theme_choice, crate::theme::Theme::Nord);
            assert!(app.theme_preview.is_none(), "kept");
        }
        send(
            &mut app,
            SettingsMessage::ProviderToggled("applications".into(), false),
        );
        assert_eq!(
            engine.providers.lock().unwrap().as_slice(),
            &[("applications".to_owned(), false)]
        );
        assert!(
            !dir.path().join("compass.json").exists(),
            "the engine writes the file, not the window"
        );
    }
}
