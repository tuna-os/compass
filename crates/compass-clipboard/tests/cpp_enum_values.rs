//! The stored enum values are read back out of the C++ header.
//!
//! WHY THIS IS NOT PARANOIA
//!
//! `kind.rs` assigns numbers to [`OfferKind`] and [`EncryptionType`], and those
//! numbers go into SQLite. Nothing in either language checks that they are the
//! *same* numbers the C++ engine uses, and for as long as both engines ship
//! side by side they read each other's rows. The failure mode is not a red
//! test: it is a history where every image is labelled a link, or — worse — an
//! offer written as `Local` that reads back as `None`, so the engine hands
//! ciphertext to the UI as though it were text.
//!
//! So the header is parsed. The C++ enums use implicit numbering (`Unknown = 0`
//! then bare names), which means the values *are* the declaration order, and
//! that is what this reads: enumerator order, compared against
//! `to_stored()`. Inserting a member in the middle of the C++ enum — the
//! natural way to add a kind, and the one that breaks every existing row —
//! fails here.
//!
//! WHAT IT CANNOT DO
//!
//! It reads text, not semantics: a member renamed but left in place still
//! matches, and a member whose *meaning* changes is invisible to it. And when
//! the C++ tree is deleted at Phase 8 this test goes with it, because by then
//! the numbering is whatever Compass says it is.

use std::path::{Path, PathBuf};

use compass_clipboard::kind::{EncryptionType, OfferKind};

const HEADER: &str = "src/server/src/services/clipboard/clipboard-db.hpp";

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/compass-clipboard sits two levels below the repository root")
        .to_path_buf()
}

fn read_header() -> String {
    let path = repo_root().join(HEADER);
    std::fs::read_to_string(&path).unwrap_or_else(|err| {
        panic!(
            "cannot read {}: {err}. If the C++ tree has moved or been deleted, this test should \
             move or be deleted with it rather than be pointed somewhere else.",
            path.display()
        )
    })
}

/// The enumerator names of `enum class <name>`, in declaration order.
///
/// Deliberately small: find the enum's opening brace, read to the closing one,
/// drop `/* ... */` comments, then take the identifier at the head of each
/// comma-separated item. Anything with an explicit `= value` is handled by
/// splitting at `=` and keeping the name — the position check below is what
/// carries the meaning, and [`explicit_values`] separately catches an explicit
/// value that disagrees with its position.
fn enumerators(text: &str, name: &str) -> Vec<String> {
    let decl = format!("enum class {name} :");
    let at = text
        .find(&decl)
        .unwrap_or_else(|| panic!("no `{decl}` in {HEADER}"));
    let open = text[at..]
        .find('{')
        .unwrap_or_else(|| panic!("`{decl}` has no body"))
        + at;
    let close = text[open..]
        .find('}')
        .unwrap_or_else(|| panic!("`{decl}`'s body is not closed"))
        + open;

    let mut body = text[open + 1..close].to_owned();
    while let Some(start) = body.find("/*") {
        let Some(end) = body[start..].find("*/") else {
            break;
        };
        body.replace_range(start..start + end + 2, "");
    }

    body.split(',')
        .map(|item| item.split('=').next().unwrap_or("").trim().to_owned())
        .filter(|item| !item.is_empty())
        .collect()
}

/// Every `Name = literal` pair in the enum body, as written.
fn explicit_values(text: &str, name: &str) -> Vec<(String, i64)> {
    let decl = format!("enum class {name} :");
    let at = text.find(&decl).expect("checked by enumerators()");
    let open = text[at..].find('{').expect("checked by enumerators()") + at;
    let close = text[open..].find('}').expect("checked by enumerators()") + open;

    text[open + 1..close]
        .split(',')
        .filter_map(|item| {
            let (lhs, rhs) = item.split_once('=')?;
            let value = rhs
                .split("/*")
                .next()?
                .trim()
                .parse::<i64>()
                .unwrap_or_else(|_| panic!("`{}` is not a plain integer", rhs.trim()));
            Some((lhs.trim().to_owned(), value))
        })
        .collect()
}

#[test]
fn the_offer_kinds_are_in_the_order_this_crate_stores_them() {
    let header = read_header();
    let cpp = enumerators(&header, "ClipboardOfferKind");

    let ours: Vec<String> = OfferKind::ALL
        .iter()
        .map(|kind| format!("{kind:?}"))
        .collect();

    assert_eq!(
        cpp, ours,
        "ClipboardOfferKind in {HEADER} no longer matches OfferKind::ALL. These are positions in \
         a stored integer column, so reordering or inserting relabels every row already on disk — \
         adding a member goes at the end, or it needs a migration."
    );

    // Position is value, because the enum numbers implicitly from `Unknown = 0`.
    for (position, kind) in OfferKind::ALL.iter().enumerate() {
        assert_eq!(
            kind.to_stored(),
            i64::try_from(position).expect("six variants"),
            "{kind:?} does not store its declaration position"
        );
    }
}

#[test]
fn the_encryption_types_are_in_the_order_this_crate_stores_them() {
    let header = read_header();
    let cpp = enumerators(&header, "ClipboardEncryptionType");

    assert_eq!(
        cpp,
        vec!["None", "Local"],
        "ClipboardEncryptionType changed"
    );
    assert_eq!(EncryptionType::None.to_stored(), 0);
    assert_eq!(EncryptionType::Local.to_stored(), 1);
}

#[test]
fn an_explicit_value_still_agrees_with_its_position() {
    // Today only `Unknown = 0` is written out. If someone adds `Pinned = 9`,
    // the order check above would still pass while the numbering diverged, so
    // the explicit values are checked against position too.
    let header = read_header();

    for (name, value) in explicit_values(&header, "ClipboardOfferKind") {
        let position = enumerators(&header, "ClipboardOfferKind")
            .iter()
            .position(|item| *item == name)
            .expect("an explicit value's name is one of the enumerators");
        assert_eq!(
            value,
            i64::try_from(position).expect("small enum"),
            "`{name} = {value}` is at position {position}, so the implicit numbering of every \
             member after it has shifted away from what this crate stores"
        );
    }
}

/// The parser has to be able to fail, or the three tests above prove nothing.
#[test]
fn the_parser_reports_what_it_reads_rather_than_what_it_hopes() {
    let fake = r"
        enum class ClipboardOfferKind : std::uint8_t {
          Unknown = 0,
          Text,
          Image, /* swapped with Link */
          Link,
          File,
          Count
        };
    ";
    assert_eq!(
        enumerators(fake, "ClipboardOfferKind"),
        vec!["Unknown", "Text", "Image", "Link", "File", "Count"],
        "the parser must see the swap; if it normalises or sorts, the real check is vacuous"
    );
    assert_ne!(
        enumerators(fake, "ClipboardOfferKind"),
        OfferKind::ALL
            .iter()
            .map(|kind| format!("{kind:?}"))
            .collect::<Vec<_>>(),
        "a swapped header must not compare equal to OfferKind::ALL"
    );

    let renumbered = r"
        enum class ClipboardOfferKind : std::uint8_t {
          Unknown = 0,
          Text,
          Link = 7,
          Image,
          File,
          Count
        };
    ";
    assert_eq!(
        explicit_values(renumbered, "ClipboardOfferKind"),
        vec![("Unknown".to_owned(), 0), ("Link".to_owned(), 7)],
        "an explicit value away from its position must be visible to the parser"
    );

    // And the comment stripping is load-bearing: without it the `/* a link ot
    // a file */` on `Count` would be read as part of an enumerator name.
    assert_eq!(
        enumerators("enum class E : int { A, B /* , C */ };", "E"),
        vec!["A", "B"],
        "a comma inside a comment must not invent an enumerator"
    );
}
