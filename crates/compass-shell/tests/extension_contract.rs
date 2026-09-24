//! The in-tree GNOME Shell extension against the contract this crate speaks.
//!
//! The extension cannot run in CI — `gnome-shell` needs logind — so the
//! drift that matters is checked statically: its copies of the interface
//! XML are the contract files, it declares the same version, it implements
//! every method and emits every signal, and `ListWindows` fills every
//! dictionary key the client decodes. A contract change that forgets the
//! extension, or an extension edit that forgets the contract, fails here.

use std::path::PathBuf;

use compass_shell::contract::{CLIPBOARD_XML, WINDOWS_XML, window_key};

fn extension_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../extensions/gnome-shell/compass@tuna-os.github.io")
}

fn read(relative: &str) -> String {
    let path = extension_dir().join(relative);
    std::fs::read_to_string(&path).unwrap_or_else(|err| panic!("{}: {err}", path.display()))
}

fn script() -> String {
    read("extension.js")
}

/// `name="..."` of every `<tag` element in `xml`.
fn names(xml: &str, tag: &str) -> Vec<String> {
    xml.split(&format!("<{tag} "))
        .skip(1)
        .filter_map(|rest| {
            let start = rest.find("name=\"")? + "name=\"".len();
            let end = rest[start..].find('"')?;
            Some(rest[start..start + end].to_owned())
        })
        .collect()
}

#[test]
fn the_extension_ships_the_contract_files_unchanged() {
    assert_eq!(
        read("dbus/org.gnome.Shell.Extensions.Vicinae.Windows.xml"),
        WINDOWS_XML
    );
    assert_eq!(
        read("dbus/org.gnome.Shell.Extensions.Vicinae.Clipboard.xml"),
        CLIPBOARD_XML
    );
}

#[test]
fn the_extension_speaks_this_contract_version() {
    let expected = format!(
        "const CONTRACT_VERSION = {};",
        compass_shell::CONTRACT_VERSION
    );
    assert!(
        script().contains(&expected),
        "extension.js must declare `{expected}`"
    );
}

#[test]
fn every_contract_method_is_implemented_and_every_signal_emitted() {
    let script = script();
    for xml in [WINDOWS_XML, CLIPBOARD_XML] {
        let methods = names(xml, "method");
        assert!(!methods.is_empty(), "the parser found no methods");
        for method in methods {
            assert!(
                script.contains(&format!("    {method}("))
                    || script.contains(&format!("    {method}Async(")),
                "extension.js does not implement {method}"
            );
        }
        // Whitespace-insensitive: a long emit wraps its arguments.
        let compact: String = script.chars().filter(|c| !c.is_whitespace()).collect();
        for signal in names(xml, "signal") {
            assert!(
                compact.contains(&format!("emit_signal('{signal}'")),
                "extension.js never emits {signal}"
            );
        }
    }
}

#[test]
fn list_windows_fills_every_key_the_client_decodes() {
    let script = script();
    for key in [
        window_key::ID,
        window_key::TITLE,
        window_key::WM_CLASS,
        window_key::WM_CLASS_INSTANCE,
        window_key::PID,
        window_key::FOCUSED,
        window_key::WORKSPACE,
        window_key::CAN_CLOSE,
    ] {
        assert!(
            script.contains(&format!("{key}: new GLib.Variant("))
                || script.contains(&format!("entry.{key} = new GLib.Variant(")),
            "ListWindows never sets `{key}`"
        );
    }
}

#[test]
fn the_extension_claims_the_gnome_versions_ci_covers() {
    let metadata = read("metadata.json");
    for version in ["\"50\"", "\"51\""] {
        assert!(
            metadata.contains(version),
            "metadata.json must list GNOME {version}"
        );
    }
}
