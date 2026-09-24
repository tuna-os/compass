//! #118 — the launcher has no accessibility tree.
//! Durable test that the gap is documented, not silent.
//! If accesskit/atspi ever appears in Cargo.lock, this test must be updated
//! alongside ADR-0016 — a green test that silently became accessible would be
//! the wrong kind of green.

use std::path::Path;

#[test]
fn a11y_gap_is_documented_not_silent() {
    let adr =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/rust-engine/adr/0016-a11y-gap.md");
    assert!(
        adr.exists(),
        "ADR-0016 must exist while the a11y gap does: {adr:?}"
    );
    let content = std::fs::read_to_string(&adr).expect("read ADR-0016");
    assert!(content.contains("Orca"));
    assert!(content.contains("accesskit"));
    assert!(content.contains("#118"));
}

#[test]
fn cargo_lock_has_no_accesskit_today() {
    // This is the condition #118 found: zero occurrences of accesskit/atspi.
    // If/when Iced gains AccessKit, this test will fail — which is the
    // intended signal to update ADR-0016 rather than silently ship it.
    let lock = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../Cargo.lock");
    let text = std::fs::read_to_string(&lock).expect("read Cargo.lock");
    let has_accesskit = text.contains("accesskit") || text.contains("atspi");
    assert!(
        !has_accesskit,
        "Cargo.lock now contains accesskit/atspi — update ADR-0016 and this test alongside the new tree, do not just delete the assertion"
    );
}
