//! The MIME type hierarchy, and walking it to find what opens a file.
//!
//! Ported from `XdgAppDatabase::findAssociations`.

use std::path::{Path, PathBuf};

use compass_xdg::mime_subclasses::{
    SUBCLASSES_PATH, Subclasses, ancestry, default_opener_through_ancestry,
    openers_through_ancestry,
};

/// The real shape of the file, as `update-mime-database` writes it.
const TABLE: &str = "\
application/x-compressed-tar application/gzip
application/gzip application/octet-stream
text/sgml text/plain
text/html text/plain
text/plain application/octet-stream
";

fn table() -> Subclasses {
    Subclasses::parse(TABLE)
}

// --- reading the table --------------------------------------------------

#[test]
fn a_child_gets_its_parent() {
    assert_eq!(
        table().parents_of("application/x-compressed-tar"),
        ["application/gzip"]
    );
}

#[test]
fn a_type_with_no_entry_has_no_parents() {
    assert!(table().parents_of("image/png").is_empty());
}

#[test]
fn a_type_may_have_several_parents() {
    // `application/x-compressed-tar` really is both a gzip and a tar.
    let subclasses = Subclasses::parse(
        "application/x-compressed-tar application/gzip\n\
         application/x-compressed-tar application/x-tar\n",
    );
    assert_eq!(
        subclasses.parents_of("application/x-compressed-tar"),
        ["application/gzip", "application/x-tar"]
    );
}

#[test]
fn a_repeated_pair_is_not_stored_twice() {
    let subclasses = Subclasses::parse("a b\na b\n");
    assert_eq!(subclasses.parents_of("a"), ["b"]);
}

#[test]
fn blank_lines_and_comments_are_skipped() {
    let subclasses = Subclasses::parse("\n# a comment\ntext/html text/plain\n\n");
    assert_eq!(subclasses.parents_of("text/html"), ["text/plain"]);

    // A two-word comment is the case that needs the `#` check: it has exactly
    // the two fields a real line has, so the field count alone would let it
    // through and register `#` as a type.
    let two_words = Subclasses::parse("# comment\na b\n");
    assert!(two_words.parents_of("#").is_empty());
    assert_eq!(two_words.parents_of("a"), ["b"]);
}

#[test]
fn a_malformed_line_is_skipped_rather_than_refusing_the_file() {
    // The file is generated from whatever packages installed, and one bad
    // line should not cost every association on the system.
    let subclasses = Subclasses::parse("text/html\nimage/png image/x-generic extra\na b\n");
    assert!(subclasses.parents_of("text/html").is_empty());
    assert!(subclasses.parents_of("image/png").is_empty());
    assert_eq!(subclasses.parents_of("a"), ["b"]);
}

#[test]
fn an_empty_table_is_empty() {
    assert!(Subclasses::parse("").is_empty());
    assert!(!table().is_empty());
}

// --- merging across data directories ------------------------------------

#[test]
fn later_directories_add_to_earlier_ones() {
    // Each package ships its own types; the union is the hierarchy.
    let mut first = Subclasses::parse("a b\n");
    first.merge(Subclasses::parse("c d\n"));
    assert_eq!(first.parents_of("a"), ["b"]);
    assert_eq!(first.parents_of("c"), ["d"]);
}

#[test]
fn a_second_parent_for_the_same_child_is_added_not_replaced() {
    let mut first = Subclasses::parse("a b\n");
    first.merge(Subclasses::parse("a c\n"));
    assert_eq!(first.parents_of("a"), ["b", "c"]);
}

#[test]
fn a_parent_already_known_is_not_repeated() {
    let mut first = Subclasses::parse("a b\n");
    first.merge(Subclasses::parse("a b\n"));
    assert_eq!(first.parents_of("a"), ["b"]);
}

#[test]
fn the_table_is_read_from_every_data_directory() {
    let dirs = vec![
        PathBuf::from("/usr/share"),
        PathBuf::from("/usr/local/share"),
    ];
    let loaded = Subclasses::load(&dirs, |path: &Path| {
        if path == Path::new("/usr/share/mime/subclasses") {
            Some("a b\n".to_owned())
        } else if path == Path::new("/usr/local/share/mime/subclasses") {
            Some("c d\n".to_owned())
        } else {
            None
        }
    });
    assert_eq!(loaded.parents_of("a"), ["b"]);
    assert_eq!(loaded.parents_of("c"), ["d"]);
}

#[test]
fn a_directory_with_no_table_is_skipped() {
    let dirs = vec![PathBuf::from("/nowhere")];
    assert!(Subclasses::load(&dirs, |_| None).is_empty());
}

#[test]
fn the_table_sits_where_the_specification_puts_it() {
    assert_eq!(SUBCLASSES_PATH, "mime/subclasses");
}

// --- walking the hierarchy ----------------------------------------------

#[test]
fn a_type_is_asked_about_before_its_parents() {
    // Which is what makes a PDF reader registered for `application/pdf` beat
    // one registered for `application/octet-stream`.
    let order = ancestry(&table(), "text/html");
    assert_eq!(order[0], "text/html");
}

#[test]
fn the_whole_chain_is_walked() {
    assert_eq!(
        ancestry(&table(), "application/x-compressed-tar"),
        [
            "application/x-compressed-tar",
            "application/gzip",
            "application/octet-stream"
        ]
    );
}

#[test]
fn a_near_ancestor_comes_before_a_distant_one() {
    let order = ancestry(&table(), "application/x-compressed-tar");
    let gzip = order.iter().position(|m| m == "application/gzip");
    let octet = order.iter().position(|m| m == "application/octet-stream");
    assert!(gzip < octet, "{order:?}");
}

#[test]
fn the_walk_is_breadth_first_across_several_parents() {
    // Both parents are asked before either parent's parent.
    let subclasses = Subclasses::parse("kid mum\nkid dad\nmum gran\ndad gramps\n");
    assert_eq!(
        ancestry(&subclasses, "kid"),
        ["kid", "mum", "dad", "gran", "gramps"]
    );
}

#[test]
fn a_type_reachable_by_two_routes_is_visited_once() {
    let subclasses = Subclasses::parse("kid mum\nkid dad\nmum gran\ndad gran\n");
    let order = ancestry(&subclasses, "kid");
    assert_eq!(order, ["kid", "mum", "dad", "gran"]);
}

#[test]
fn a_cycle_in_the_table_terminates() {
    // A declared divergence. The C++ keeps no visited set, so a cycle loops
    // forever. `subclasses` is generated, so a cycle would be a bug in
    // `update-mime-database` or in a package's XML — but it is still a file on
    // disk that this reads, and a launcher that hangs on a malformed system
    // file is worse than one that copes.
    let subclasses = Subclasses::parse("a b\nb c\nc a\n");
    assert_eq!(ancestry(&subclasses, "a"), ["a", "b", "c"]);
}

#[test]
fn a_self_referential_type_terminates_too() {
    let subclasses = Subclasses::parse("a a\n");
    assert_eq!(ancestry(&subclasses, "a"), ["a"]);
}

#[test]
fn an_unknown_type_is_its_own_whole_ancestry() {
    assert_eq!(ancestry(&table(), "image/png"), ["image/png"]);
}

// --- finding openers ----------------------------------------------------

fn registry(pairs: &[(&'static str, &'static [&'static str])]) -> impl Fn(&str) -> Vec<String> {
    let owned: Vec<(String, Vec<String>)> = pairs
        .iter()
        .map(|(mime, ids)| {
            (
                (*mime).to_owned(),
                ids.iter().map(|id| (*id).to_owned()).collect(),
            )
        })
        .collect();
    move |mime: &str| {
        owned
            .iter()
            .find(|(known, _)| known == mime)
            .map_or_else(Vec::new, |(_, ids)| ids.clone())
    }
}

#[test]
fn something_registered_for_the_type_itself_comes_first() {
    let openers = openers_through_ancestry(
        &table(),
        "text/html",
        registry(&[
            ("text/html", &["browser.desktop"]),
            ("text/plain", &["editor.desktop"]),
        ]),
    );
    assert_eq!(openers, ["browser.desktop", "editor.desktop"]);
}

#[test]
fn a_parents_opener_is_found_when_nothing_claims_the_type() {
    // The case this whole module exists for: a `.tar.gz` finds the archive
    // manager that registered for `application/gzip`.
    let openers = openers_through_ancestry(
        &table(),
        "application/x-compressed-tar",
        registry(&[("application/gzip", &["archiver.desktop"])]),
    );
    assert_eq!(openers, ["archiver.desktop"]);
}

#[test]
fn an_application_claiming_both_appears_once_in_its_nearest_position() {
    // Its nearest claim is the one that says most about what it can do.
    let openers = openers_through_ancestry(
        &table(),
        "text/html",
        registry(&[
            ("text/html", &["both.desktop"]),
            ("text/plain", &["editor.desktop", "both.desktop"]),
        ]),
    );
    assert_eq!(openers, ["both.desktop", "editor.desktop"]);
}

#[test]
fn nothing_anywhere_in_the_chain_finds_nothing() {
    let openers = openers_through_ancestry(
        &table(),
        "text/html",
        registry(&[("image/png", &["viewer"])]),
    );
    assert!(openers.is_empty());
}

#[test]
fn the_default_is_the_first_of_the_walk() {
    let chosen = default_opener_through_ancestry(
        &table(),
        "text/html",
        registry(&[
            ("text/html", &["browser.desktop"]),
            ("text/plain", &["editor.desktop"]),
        ]),
    );
    assert_eq!(chosen.as_deref(), Some("browser.desktop"));
}

#[test]
fn the_default_falls_back_to_a_parents_opener() {
    let chosen = default_opener_through_ancestry(
        &table(),
        "application/x-compressed-tar",
        registry(&[("application/octet-stream", &["hexedit.desktop"])]),
    );
    assert_eq!(chosen.as_deref(), Some("hexedit.desktop"));
}

#[test]
fn nothing_in_the_chain_means_no_default() {
    assert_eq!(
        default_opener_through_ancestry(&table(), "text/html", registry(&[])),
        None
    );
}
