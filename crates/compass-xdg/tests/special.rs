//! Desktop files from the wild: `src/lib/xdgpp/tests/special.cpp`, ported.
//!
//! Each case is a real file someone reported, which is why they are kept as
//! they are rather than reduced to the one key they exercise.

use compass_xdg::{DesktopEntry, ParseOptions, locale::Locale};

// https://github.com/vicinaehq/vicinae/discussions/145
#[test]
fn a_wine_generated_entry_unescapes_its_windows_path() {
    let entry = DesktopEntry::parse(
        r#"
[Desktop Entry]
Name=한워드 2022
Exec=env WINEPREFIX="/home/quadratech/.wine" wine C:\\\\users\\\\quadratech\\\\AppData\\\\Roaming\\\\Microsoft\\\\Windows\\\\Start\\ Menu\\\\Programs\\\\한워드\\ 2022.lnk
Type=Application
StartupNotify=true
Comment=호환성이 높은 워드프로세서 문서를 만듭니다.
Path=/home/quadratech/.wine/dosdevices/c:/Program Files (x86)/Hnc/Office 2022/HOffice120/Bin/
Icon=ACEB_HWord.0
StartupWMClass=hword.exe
	"#,
    )
    .expect("parses");
    let exec = entry.expand_exec();
    assert_eq!(exec[0], "env");
    assert_eq!(exec[1], "WINEPREFIX=/home/quadratech/.wine");
    assert_eq!(exec[2], "wine");
    assert_eq!(
        exec[3],
        "C:\\users\\quadratech\\AppData\\Roaming\\Microsoft\\Windows\\Start Menu\\Programs\\한워드 2022.lnk"
    );
}

// Not what the specification allows, and supported on purpose: web-app
// launchers write single-quoted arguments.
#[test]
fn single_quotes_quote_an_argument() {
    let entry = DesktopEntry::parse(
        r"
[Desktop Entry]
Type=Application
Name=YouTube Music
Exec=vivaldi --app='https://music.youtube.com'
Icon=ftwa-youtube-music
Terminal=false
StartupNotify=true
StartupWMClass=ftwa-youtube-music
",
    )
    .expect("parses");
    let exec = entry.expand_exec();
    assert_eq!(exec[0], "vivaldi");
    assert_eq!(exec[1], "--app=https://music.youtube.com");
}

#[test]
fn empty_values_are_empty_rather_than_missing_or_swallowing_the_next_line() {
    let entry = DesktopEntry::parse(
        r"
[Desktop Entry]
Categories=Network;WebBrowser;
Comment=
Exec=zen %U
GenericName=A sleek browser
Icon=/home/user/icons/Zen128.png
MimeType=text/html;text/xml;application/xhtml+xml;application/vnd.mozilla.xul+xml;text/mml;x-scheme-handler/http;x-scheme-handler/https;
Name=Zen Browser
Path=
StartupNotify=true
StartupWMClass=zen-beta
Terminal=false
TerminalOptions=
Type=Application
Version=1.0
X-KDE-SubstituteUID=false
X-KDE-Username=
",
    )
    .expect("parses");
    assert_eq!(entry.comment().unwrap_or_default(), "");
    assert_eq!(
        entry
            .working_directory()
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_default(),
        ""
    );
    assert_eq!(entry.exec(), Some("zen %U"));
}

#[test]
fn a_locale_with_no_translation_of_its_own_gets_the_plain_name() {
    // An excerpt of the reported file's 86 translations: the ones near
    // `en_US`, which is what the lookup could wrongly land on.
    let entry = DesktopEntry::parse_with(
        r"
[Desktop Entry]
Name[el]=Βίντεο
Name[en_GB]=Videos
Name[en@shaw]=𐑝𐑦𐑛𐑰𐑴𐑟
Name[eo]=Videaĵoj
Name[es]=Vídeos
Name=Videos
Exec=totem %U
# Translators: Do NOT translate or transliterate this text (this is an icon file name)!
Icon=org.gnome.Totem
DBusActivatable=true
Terminal=false
Type=Application
Categories=GTK;GNOME;AudioVideo;Player;Video;
X-GNOME-DocPath=totem/totem.xml
StartupNotify=true
",
        &ParseOptions {
            locale: Some(Locale::parse("en_US.utf8")),
            path: None,
        },
    )
    .expect("parses");
    assert_eq!(entry.name(), "Videos");
    assert_eq!(entry.expand_exec(), ["totem"]);
}
