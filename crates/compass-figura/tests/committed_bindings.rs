//! The TypeScript bindings are committed, so building the extension runtime
//! needs npm and nothing else. This is what keeps them honest: an IDL change
//! without `make figen` fails here, not at runtime inside an extension.

use std::path::{Path, PathBuf};

use compass_figura::{COMMITTED, compile};

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/compass-figura sits two levels below the repository root")
        .to_path_buf()
}

fn read(rel: &str) -> String {
    let path = repo().join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

#[test]
fn every_committed_binding_is_what_its_idl_generates() {
    let mut stale = Vec::new();
    for (fig, side, out) in COMMITTED {
        let generated = compile(&read(fig), *side).unwrap_or_else(|e| panic!("{fig}: {e}"));
        if generated != read(out) {
            stale.push(*out);
        }
    }
    assert!(
        stale.is_empty(),
        "stale bindings (run `make figen` and commit the result): {stale:?}"
    );
}

#[test]
fn every_idl_in_the_repository_parses() {
    let dir = repo().join("figura");
    let mut count = 0;
    for entry in std::fs::read_dir(&dir).expect("figura/ exists") {
        let path = entry.expect("a directory entry").path();
        if path.extension().is_some_and(|ext| ext == "fig") {
            let source = std::fs::read_to_string(&path).expect("readable");
            let tree = compass_figura::parse(&source)
                .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            assert!(
                !tree.services.is_empty(),
                "{} declares no service",
                path.display()
            );
            count += 1;
        }
    }
    assert!(count >= 4, "only {count} .fig files found");
}
