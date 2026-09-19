//! The C++ test suite, replayed through the Rust parser.
//!
//! `src/lib/script-command/tests/` holds 73 Catch2 cases, each of which is a
//! script in a raw string literal plus an expectation about whether it parses.
//! Building that suite needs CMake, Catch2 and glaze; reading it needs none of
//! them, and the scripts in it are the closest thing this port has to a corpus
//! written by someone other than itself.
//!
//! So this extracts every `R"(...)"` source together with the `REQUIRE` that
//! follows it, and asserts the Rust parser agrees about accept-or-reject. It
//! deliberately does **not** compare field values: those assertions are C++
//! expressions, and a harness that tried to interpret them would be a small
//! broken C++ interpreter. Field-level parity is covered by the unit tests in
//! `script_command.rs`, which were written from the C++ source.
//!
//! When the C++ suite gains a case, this gains a case.

use std::path::{Path, PathBuf};

use compass_core::script_command::ScriptCommand;

fn tests_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("two levels below the repository root")
        .join("src/lib/script-command/tests")
}

/// One extracted case.
#[derive(Debug)]
struct Case {
    file: String,
    name: String,
    source: String,
    /// What the C++ expects: `true` for `has_value()`, `false` for the
    /// negated form.
    accepts: bool,
}

/// Every `TEST_CASE` that contains a raw-string script and an expectation.
fn cases() -> Vec<Case> {
    let mut cases = Vec::new();

    for entry in std::fs::read_dir(tests_dir()).expect("the C++ test directory") {
        let path = entry.expect("a directory entry").path();
        if path.extension().is_none_or(|e| e != "cpp") {
            continue;
        }
        let file = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let text = std::fs::read_to_string(&path).expect("read the test file");

        for block in text.split("TEST_CASE(").skip(1) {
            let name = block.split('"').nth(1).unwrap_or("<unnamed>").to_owned();

            // The script itself. Several cases declare more than one source;
            // each is checked.
            let mut rest = block;
            while let Some(start) = rest.find("R\"(") {
                let after = &rest[start + 3..];
                let Some(end) = after.find(")\"") else {
                    break;
                };
                let source = after[..end].to_owned();
                let tail = &after[end..];
                rest = tail;

                // The first REQUIRE after this source says what is expected of
                // it. `!...has_value()` and `REQUIRE_FALSE` are both refusals.
                let Some(require_at) = tail.find("REQUIRE") else {
                    break;
                };
                let line: String = tail[require_at..]
                    .chars()
                    .take_while(|c| *c != ';')
                    .collect();
                // `REQUIRE(!result.has_value())` and `REQUIRE_FALSE(...)`
                // are both refusals; so is `REQUIRE(!script_command::...)`.
                // The `!` is what matters and it sits right after the paren.
                let argument = line
                    .split_once('(')
                    .map(|(_, rest)| rest.trim_start())
                    .unwrap_or_default();
                let accepts = !(line.contains("REQUIRE_FALSE") || argument.starts_with('!'));

                cases.push(Case {
                    file: file.clone(),
                    name: name.clone(),
                    source,
                    accepts,
                });
            }
        }
    }

    cases
}

#[test]
fn the_corpus_is_large_enough_to_be_worth_checking() {
    // A harness that silently extracted nothing would pass for ever. The C++
    // suite has 73 TEST_CASEs today, most with one script in them.
    let cases = cases();
    assert!(
        cases.len() >= 50,
        "extracted only {} cases from the C++ tests; the extractor has drifted from their shape",
        cases.len()
    );
    let rejections = cases.iter().filter(|case| !case.accepts).count();
    assert!(
        rejections >= 10,
        "extracted only {rejections} rejection cases; the C++ suite has more than that, so the \
         negative half is not being checked"
    );
    assert!(
        cases.iter().any(|case| case.accepts),
        "no acceptance cases were extracted"
    );
}

#[test]
fn the_rust_parser_accepts_and_rejects_what_the_cpp_suite_says_it_should() {
    let mut disagreements = Vec::new();

    for case in cases() {
        let parsed = ScriptCommand::parse(&case.source);
        if parsed.is_ok() != case.accepts {
            disagreements.push(format!(
                "{} / {}: C++ {}, Rust {} ({})",
                case.file,
                case.name,
                if case.accepts { "accepts" } else { "rejects" },
                if parsed.is_ok() { "accepts" } else { "rejects" },
                parsed.err().unwrap_or_else(|| "parsed".to_owned()),
            ));
        }
    }

    assert!(
        disagreements.is_empty(),
        "the two parsers disagree about {} of the C++ suite's own scripts:\n{}",
        disagreements.len(),
        disagreements.join("\n")
    );
}

#[test]
fn how_many_cases_the_corpus_holds() {
    // Printed rather than asserted beyond the floor above: it is a figure for
    // the commit message and the roadmap, and pinning it would fail on every
    // case the C++ suite gains.
    let cases = cases();
    let rejections = cases.iter().filter(|case| !case.accepts).count();
    println!(
        "script-command corpus: {} cases ({} accept, {rejections} reject) from {} files",
        cases.len(),
        cases.len() - rejections,
        std::fs::read_dir(tests_dir())
            .expect("the C++ test directory")
            .filter_map(Result::ok)
            .filter(|e| e.path().extension().is_some_and(|x| x == "cpp"))
            .count(),
    );
}
