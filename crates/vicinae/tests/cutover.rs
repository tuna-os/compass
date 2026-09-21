//! Phase 7 cutover — durable checks that the default and narrowing are stated.

use std::path::Path;

#[test]
fn rust_is_default_on_linux() {
    use clap::Parser;
    use vicinae::cli::Cli;
    use vicinae::engine::Engine;
    let cli = Cli::try_parse_from(["vicinae", "ping"]).expect("parse");
    assert_eq!(cli.engine, Engine::Rust, "Rust must be default (Phase 7)");
}

#[test]
fn cpp_escape_hatch_still_parses() {
    use clap::Parser;
    use vicinae::cli::Cli;
    use vicinae::engine::Engine;
    let cli = Cli::try_parse_from(["vicinae", "--engine", "cpp", "ping"]).expect("parse cpp");
    assert_eq!(cli.engine, Engine::Cpp);
}

#[test]
fn cutover_doc_states_narrowing() {
    let doc = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/rust-engine/CUTOVER.md");
    let text = std::fs::read_to_string(&doc).expect("CUTOVER.md must exist for #10");
    assert!(
        text.contains("macOS and Windows"),
        "must state non-Linux keeps C++"
    );
    assert!(text.contains("Orca") || text.contains("accessibility") || text.contains("a11y"));
    assert!(text.contains("ADR-0016"));
    assert!(text.contains("768×608") || text.contains("768x608"));
}
