//! Store-listing screenshots, rendered through the paint tier.
//!
//! Ignored by default: `just screenshots` runs it and writes PNGs into
//! `packaging/screenshots/`. The desktop entries are a fixed demo set so the
//! pictures do not depend on what the machine running it has installed.

use compass_core::apps::AppIndex;
use compass_ui::LauncherApp;
use compass_ui::design::Appearance;
use compass_ui::message::Message;

const DEMO_APPS: [(&str, &str); 8] = [
    ("Files", "system-file-manager"),
    ("Firefox", "web-browser"),
    ("Terminal", "utilities-terminal"),
    ("Text Editor", "accessories-text-editor"),
    ("Settings", "preferences-system"),
    ("Calculator", "accessories-calculator"),
    ("Games", "applications-games"),
    ("Documents", "x-office-document"),
];

fn launcher(dir: &std::path::Path, appearance: Appearance) -> LauncherApp {
    for (name, icon) in DEMO_APPS {
        std::fs::write(
            dir.join(format!("{}.desktop", name.replace(' ', "-"))),
            format!(
                "[Desktop Entry]\nType=Application\nName={name}\nIcon={icon}\nExec=/bin/true\n"
            ),
        )
        .expect("write a desktop entry");
    }
    let mut app = LauncherApp::with_index(AppIndex::builder().dir(dir).build());
    let _ = app.update(Message::AppearanceChanged(appearance));
    app
}

fn shoot(app: &LauncherApp, out: &std::path::Path, name: &str) {
    let mut ui = iced_test::Simulator::with_size(
        iced::Settings::default(),
        iced::Size::new(760.0, 480.0),
        app.view(),
    );
    let snapshot = ui.snapshot(&app.theme()).expect("the frame renders");
    let tmp = tempfile::tempdir().expect("tempdir");
    snapshot
        .matches_image(tmp.path().join(name))
        .expect("written");
    let png = std::fs::read_dir(tmp.path())
        .expect("dir")
        .find_map(|e| e.ok().map(|e| e.path()))
        .expect("a png");
    std::fs::copy(png, out.join(format!("{name}.png"))).expect("copy");
}

#[test]
#[ignore = "writes store screenshots; run with just screenshots"]
fn store_screenshots() {
    let out = std::path::PathBuf::from(
        std::env::var("SCREENSHOTS_DIR").unwrap_or_else(|_| "../../packaging/screenshots".into()),
    );
    std::fs::create_dir_all(&out).expect("output dir");
    let dir = tempfile::tempdir().expect("tempdir");

    let mut app = launcher(dir.path(), Appearance::Dark);
    let _ = app.update(Message::QueryChanged("term".to_owned()));
    shoot(&app, &out, "search");

    let mut app = launcher(dir.path(), Appearance::Dark);
    let _ = app.update(Message::QueryChanged("sqrt(2) * 12 kg to lb".to_owned()));
    shoot(&app, &out, "calculator");

    let mut app = launcher(dir.path(), Appearance::Light);
    let _ = app.update(Message::QueryChanged("Open Settings".to_owned()));
    let _ = app.update(Message::LaunchSelected);
    shoot(&app, &out, "settings");

    let mut app = launcher(dir.path(), Appearance::Dark);
    let _ = app.update(Message::QueryChanged("Search Emojis".to_owned()));
    let _ = app.update(Message::LaunchSelected);
    let _ = app.update(Message::EmojiQueryChanged("heart".to_owned()));
    shoot(&app, &out, "emoji");
}
