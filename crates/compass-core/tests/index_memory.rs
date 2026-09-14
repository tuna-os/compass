//! What an index of 10,000 desktop entries costs in memory.
//!
//! §8.5 names "Peak RSS, 10k index + 3 extensions — < 150 MB" and marks it
//! *new*: proposed by the plan rather than inherited from the gist spec. Like
//! every other row in that table it had no measurement of any kind — the audit
//! in §8.5 found exactly one benchmark in the workspace and no CI job running
//! benches.
//!
//! **This measures the index half only, and cannot measure the other.** There
//! is no extension host yet; that is Phase 4. So this does not evaluate the
//! SLA. It puts a number on the part that exists, so that when the host arrives
//! the remaining budget is known rather than guessed.
//!
//! VmHWM, not VmRSS: the SLA says *peak*, and RSS at the moment you happen to
//! read it is not a peak. VmHWM is the kernel's high-water mark for the
//! process, which is the quantity the row is about.

use std::fmt::Write as _;
use std::fs;

use compass_core::apps::AppIndex;
use tempfile::tempdir;

const ENTRIES: usize = 10_000;

/// The process's high-water resident set, in kilobytes.
///
/// Linux-only, and the test says so rather than silently passing elsewhere: a
/// memory assertion that quietly evaporates on another platform is the kind of
/// check this project has had to fix repeatedly.
fn peak_rss_kb() -> Option<u64> {
    let status = fs::read_to_string("/proc/self/status").ok()?;
    status
        .lines()
        .find_map(|line| line.strip_prefix("VmHWM:"))
        .and_then(|rest| rest.split_whitespace().next()?.parse().ok())
}

#[test]
fn an_index_of_ten_thousand_entries_reports_its_cost() {
    let Some(before) = peak_rss_kb() else {
        println!("skipping: /proc/self/status is unreadable, so this is not Linux");
        return;
    };

    let dir = tempdir().expect("temp dir");

    // Written to disk rather than synthesised in memory, because AppIndex reads
    // directories and a fake in-memory path would measure a different thing
    // from the one the SLA is about.
    let mut body = String::new();
    for i in 0..ENTRIES {
        body.clear();
        write!(
            body,
            "[Desktop Entry]\n\
             Type=Application\n\
             Name=Generated Application {i}\n\
             GenericName=Example {i}\n\
             Comment=A synthetic entry standing in for a real one\n\
             Exec=/usr/bin/true --instance {i}\n\
             Icon=application-x-executable\n\
             Categories=Utility;\n"
        )
        .expect("format");
        fs::write(dir.path().join(format!("generated-{i}.desktop")), &body).expect("write");
    }

    let index = AppIndex::builder().dir(dir.path()).build();
    let indexed = index.applications().count();
    assert_eq!(
        indexed, ENTRIES,
        "the index dropped entries: {indexed} of {ENTRIES}"
    );

    let after = peak_rss_kb().expect("VmHWM readable after it was readable before");
    let delta_kb = after.saturating_sub(before);

    println!(
        "index of {ENTRIES} entries: peak RSS {before} kB -> {after} kB, delta {delta_kb} kB \
         ({:.1} MB, {:.0} bytes per entry)",
        delta_kb as f64 / 1024.0,
        (delta_kb as f64 * 1024.0) / ENTRIES as f64
    );

    // Deliberately loose, and not the SLA.
    //
    // The SLA is 150 MB for the index PLUS three extensions, and the extension
    // host does not exist. Asserting 150 MB here would quietly convert a
    // whole-system budget into an index-only one and call it met — the same
    // error as reading a green tick on a check that measures the wrong thing.
    //
    // 100 MB catches an index that has gone catastrophically wrong while
    // leaving the real budget to whoever can measure all of it.
    const SANITY_CEILING_KB: u64 = 100 * 1024;
    assert!(
        delta_kb < SANITY_CEILING_KB,
        "indexing {ENTRIES} entries grew peak RSS by {delta_kb} kB, past the {SANITY_CEILING_KB} kB \
         sanity ceiling. This is NOT the §8.5 SLA — that is 150 MB for the index plus three \
         extensions, and the extension host does not exist yet."
    );
}
