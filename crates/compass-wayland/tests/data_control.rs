//! Which clipboard offers are kept, and which of them are read.
//!
//! Ported from `src/data-control-server/src/selection.cpp` and
//! `src/data-control-server/src/wayland/mime.hpp`.

use std::cell::RefCell;
use std::collections::BTreeSet;

use compass_wayland::data_control::{
    CONCEALED_MIME_TYPE, MAX_PRIMARY_SIZE, Offer, PASSWORD_HINT_MIME_TYPE, build_primary_selection,
    build_selection, filter_mimes, is_data_mime, is_flag_mime,
};

fn mimes(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| (*s).to_owned()).collect()
}

fn kept(list: &[&str]) -> Vec<String> {
    filter_mimes(&mimes(list)).into_iter().collect()
}

/// A receiver that records every type whose bytes were asked for.
struct Recorder {
    asked: RefCell<Vec<String>>,
}

impl Recorder {
    fn new() -> Self {
        Self {
            asked: RefCell::new(Vec::new()),
        }
    }

    fn receive(&self, mime: &str) -> Vec<u8> {
        self.asked.borrow_mut().push(mime.to_owned());
        format!("<{mime}>").into_bytes()
    }
}

// --- what counts as data ------------------------------------------------

#[test]
fn text_image_and_application_types_are_data() {
    assert!(is_data_mime("text/html"));
    assert!(is_data_mime("image/png"));
    assert!(is_data_mime("application/pdf"));
}

#[test]
fn the_gnome_file_list_is_kept_despite_its_prefix() {
    // `x-special/` matches none of the accepted prefixes, so it is named
    // outright — it is how a file manager says "these files were copied".
    assert!(is_data_mime("x-special/gnome-copied-files"));
}

#[test]
fn an_unknown_prefix_is_not_data() {
    assert!(!is_data_mime("x-special/something-else"));
    assert!(!is_data_mime("chemical/x-pdb"));
}

#[test]
fn the_qt_image_hint_is_dropped_although_its_prefix_matches() {
    // It is a hint that an image exists, and the filter already keeps exactly
    // one image encoding, so the hint is noise. The ignore list is checked
    // first for precisely this reason.
    assert!(!is_data_mime("application/x-qt-image"));
}

#[test]
fn the_flag_types_are_flags_and_not_data() {
    assert!(is_flag_mime(PASSWORD_HINT_MIME_TYPE));
    assert!(is_flag_mime(CONCEALED_MIME_TYPE));
    assert!(!is_data_mime(PASSWORD_HINT_MIME_TYPE));
    assert!(!is_data_mime(CONCEALED_MIME_TYPE));
}

#[test]
fn a_flag_type_is_kept_even_though_it_is_not_data() {
    // The filter accepts a type that is *either* a flag or data, which is the
    // only way a password hint survives — its prefix matches nothing.
    assert_eq!(
        kept(&[PASSWORD_HINT_MIME_TYPE, "text/plain"]),
        vec!["text/plain".to_owned(), PASSWORD_HINT_MIME_TYPE.to_owned()]
    );
}

#[test]
fn the_kept_types_come_back_sorted_not_in_offer_order() {
    // The C++ collects into a `std::set<std::string>`, so the entry's offers
    // are in lexicographic order however the client listed them. Anything
    // downstream that reads `offers[0]` is reading the alphabetically first
    // type, not the client's first choice.
    //
    // No mutation of `filter_mimes` can make this test fail: the `BTreeSet`
    // return type is what sorts, so the guarantee is in the type rather than
    // in code that could drift. It is kept as the record of a fact a reader
    // would otherwise have to infer, and it would catch a later change of that
    // return type to something order-preserving.
    assert_eq!(
        kept(&["text/plain", "image/png", "application/pdf"]),
        vec!["application/pdf", "image/png", "text/plain"]
    );
}

#[test]
fn a_type_that_is_neither_is_dropped() {
    assert_eq!(kept(&["chemical/x-pdb", "text/plain"]), vec!["text/plain"]);
}

// --- picking one image --------------------------------------------------

#[test]
fn only_one_image_encoding_survives() {
    let out = kept(&["image/png", "image/jpeg", "image/webp"]);
    assert_eq!(out, vec!["image/png"]);
}

#[test]
fn the_best_ranked_encoding_wins_regardless_of_offer_order() {
    // A screenshot tool may list its encodings in any order; the preference
    // list decides, not the client.
    assert_eq!(kept(&["image/webp", "image/gif"]), vec!["image/gif"]);
    assert_eq!(kept(&["image/gif", "image/webp"]), vec!["image/gif"]);
}

#[test]
fn a_known_encoding_displaces_an_unknown_one() {
    assert_eq!(kept(&["image/bmp", "image/png"]), vec!["image/png"]);
}

#[test]
fn an_unknown_encoding_never_displaces_a_known_one() {
    assert_eq!(kept(&["image/png", "image/bmp"]), vec!["image/png"]);
}

#[test]
fn two_unknown_encodings_keep_the_first_offered() {
    // Both score the same, and the comparison demands a *strictly* better
    // score to displace — so the result does not depend on which the client
    // happened to list second.
    assert_eq!(kept(&["image/bmp", "image/tiff"]), vec!["image/bmp"]);
    assert_eq!(kept(&["image/tiff", "image/bmp"]), vec!["image/tiff"]);
}

#[test]
fn the_displaced_encoding_is_removed_rather_than_left_behind() {
    // The saved type is erased from the set before the better one is inserted;
    // without that, the entry would carry both encodings of one image.
    let out = kept(&["image/webp", "image/png", "image/gif"]);
    assert_eq!(out, vec!["image/gif"]);
}

#[test]
fn images_do_not_displace_text() {
    let out = kept(&["image/png", "text/html", "image/jpeg"]);
    assert_eq!(out, vec!["image/png", "text/html"]);
}

// --- the two plain-text spellings ---------------------------------------

#[test]
fn the_unlabelled_plain_text_goes_when_the_utf8_one_is_there() {
    assert_eq!(
        kept(&["text/plain", "text/plain;charset=utf-8"]),
        vec!["text/plain;charset=utf-8"]
    );
}

#[test]
fn plain_text_alone_is_kept() {
    assert_eq!(kept(&["text/plain"]), vec!["text/plain"]);
}

#[test]
fn a_different_charset_does_not_displace_plain_text() {
    // Only the UTF-8 spelling is treated as saying the same thing better.
    let out = kept(&["text/plain", "text/plain;charset=iso-8859-1"]);
    assert_eq!(out, vec!["text/plain", "text/plain;charset=iso-8859-1"]);
}

// --- reading the bytes --------------------------------------------------

fn set(list: &[&str]) -> BTreeSet<String> {
    list.iter().map(|s| (*s).to_owned()).collect()
}

#[test]
fn every_kept_type_is_read() {
    let rec = Recorder::new();
    let selection = build_selection(&set(&["text/html", "text/plain"]), |m| rec.receive(m));
    assert_eq!(
        selection.offers,
        vec![
            Offer {
                mime_type: "text/html".to_owned(),
                data: b"<text/html>".to_vec()
            },
            Offer {
                mime_type: "text/plain".to_owned(),
                data: b"<text/plain>".to_vec()
            },
        ]
    );
}

#[test]
fn a_flag_type_is_recorded_without_ever_being_read() {
    // Reading it would mean opening a pipe to the client for a type that was
    // never meant to carry contents.
    let rec = Recorder::new();
    let selection = build_selection(&set(&[PASSWORD_HINT_MIME_TYPE, "text/plain"]), |m| {
        rec.receive(m)
    });
    assert_eq!(rec.asked.borrow().as_slice(), ["text/plain"]);
    let flag = selection
        .offers
        .iter()
        .find(|o| o.mime_type == PASSWORD_HINT_MIME_TYPE)
        .expect("the flag is kept");
    assert!(flag.data.is_empty());
}

#[test]
fn an_empty_selection_reads_nothing() {
    let rec = Recorder::new();
    let selection = build_selection(&set(&[]), |m| rec.receive(m));
    assert!(selection.offers.is_empty());
    assert!(rec.asked.borrow().is_empty());
}

// --- the primary selection ----------------------------------------------

#[test]
fn the_primary_selection_keeps_utf8_text_first() {
    let rec = Recorder::new();
    let selection =
        build_primary_selection(&mimes(&["text/plain", "text/plain;charset=utf-8"]), |m| {
            rec.receive(m)
        });
    assert_eq!(rec.asked.borrow().as_slice(), ["text/plain;charset=utf-8"]);
    assert_eq!(selection.offers.len(), 1);
    assert_eq!(selection.offers[0].mime_type, "text/plain;charset=utf-8");
}

#[test]
fn the_primary_selection_falls_back_to_unlabelled_text() {
    let rec = Recorder::new();
    let selection = build_primary_selection(&mimes(&["text/plain"]), |m| rec.receive(m));
    assert_eq!(selection.offers.len(), 1);
    assert_eq!(selection.offers[0].mime_type, "text/plain");
}

#[test]
fn the_primary_selection_stops_at_the_first_spelling_it_finds() {
    // The loop breaks, so a selection never carries both spellings of the same
    // text — and the second is never read.
    let rec = Recorder::new();
    build_primary_selection(&mimes(&["text/plain;charset=utf-8", "text/plain"]), |m| {
        rec.receive(m)
    });
    assert_eq!(rec.asked.borrow().len(), 1);
}

#[test]
fn a_concealed_primary_selection_is_dropped_whole() {
    let rec = Recorder::new();
    let selection = build_primary_selection(&mimes(&[CONCEALED_MIME_TYPE, "text/plain"]), |m| {
        rec.receive(m)
    });
    assert!(selection.offers.is_empty());
    assert!(
        rec.asked.borrow().is_empty(),
        "a concealed selection must not be read at all"
    );
}

#[test]
fn a_password_hint_does_not_conceal_the_primary_selection() {
    // Only the concealed type stops the primary selection. The password hint
    // is checked in the clipboard path, not here.
    let selection =
        build_primary_selection(&mimes(&[PASSWORD_HINT_MIME_TYPE, "text/plain"]), |_| {
            b"secret".to_vec()
        });
    assert_eq!(selection.offers.len(), 1);
}

#[test]
fn the_primary_selection_keeps_nothing_but_text() {
    // An image dragged out with the middle-click buffer is not recorded.
    let rec = Recorder::new();
    let selection =
        build_primary_selection(&mimes(&["image/png", "text/html"]), |m| rec.receive(m));
    assert!(selection.offers.is_empty());
    assert!(rec.asked.borrow().is_empty());
}

#[test]
fn an_oversized_primary_selection_is_dropped_rather_than_truncated() {
    let big = vec![b'x'; MAX_PRIMARY_SIZE + 1];
    let selection = build_primary_selection(&mimes(&["text/plain"]), |_| big.clone());
    assert!(selection.offers.is_empty());
}

#[test]
fn a_primary_selection_exactly_at_the_limit_is_kept() {
    // The comparison is strictly greater, so the limit itself is allowed.
    let at_limit = vec![b'x'; MAX_PRIMARY_SIZE];
    let selection = build_primary_selection(&mimes(&["text/plain"]), |_| at_limit.clone());
    assert_eq!(selection.offers.len(), 1);
    assert_eq!(selection.offers[0].data.len(), MAX_PRIMARY_SIZE);
}

#[test]
fn an_oversized_selection_is_not_retried_with_the_other_spelling() {
    // The size check returns, it does not continue — so an oversized UTF-8
    // selection does not fall back to reading the same text again unlabelled.
    let rec = Recorder::new();
    let selection =
        build_primary_selection(&mimes(&["text/plain;charset=utf-8", "text/plain"]), |m| {
            rec.receive(m);
            vec![b'x'; MAX_PRIMARY_SIZE + 1]
        });
    assert!(selection.offers.is_empty());
    assert_eq!(rec.asked.borrow().len(), 1);
}

#[test]
fn the_primary_size_limit_is_one_mebibyte() {
    // Pinned to the literal, because every other test here measures against
    // the constant and would follow it if it changed.
    assert_eq!(MAX_PRIMARY_SIZE, 1_048_576);
}
