//! Discovery on disk and hot reload.

use std::path::Path;
use std::time::Duration;

use compass_extension_api::ExtensionId;
use compass_script::{ScriptSet, ScriptWatcher, discovery};

fn write_script(root: &Path, name: &str, body: &str) {
    let dir = root.join(name);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("script.toml"), format!("title = \"{name}\"")).unwrap();
    std::fs::write(dir.join("main.rhai"), body).unwrap();
}

fn names(ids: &[ExtensionId]) -> Vec<&str> {
    ids.iter().map(ExtensionId::as_str).collect()
}

#[test]
fn a_reload_reports_exactly_what_was_added_changed_and_removed() {
    let root = tempfile::tempdir().unwrap();
    let roots = [root.path().to_path_buf()];
    write_script(root.path(), "a", "fn search(q) { [] }");
    write_script(root.path(), "b", "fn search(q) { [] }");

    let mut set = ScriptSet::new();
    let first = set.apply(discovery::scan(&roots));
    assert_eq!(names(&first.added), ["script.a", "script.b"]);
    assert!(
        set.apply(discovery::scan(&roots)).is_empty(),
        "no change, no reload"
    );

    write_script(root.path(), "a", "fn search(q) { [#{ title: q }] }");
    std::fs::remove_dir_all(root.path().join("b")).unwrap();
    write_script(root.path(), "c", "fn search(q) { [] }");
    let reload = set.apply(discovery::scan(&roots));
    assert_eq!(names(&reload.added), ["script.c"]);
    assert_eq!(names(&reload.changed), ["script.a"]);
    assert_eq!(names(&reload.removed), ["script.b"]);
    assert_eq!(set.len(), 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_watcher_notices_an_edit_once_the_burst_settles() {
    let root = tempfile::tempdir().unwrap();
    write_script(root.path(), "a", "fn search(q) { [] }");
    let missing = root.path().join("not-there");
    let mut watcher = ScriptWatcher::new(&[root.path().to_path_buf(), missing]).unwrap();
    assert_eq!(watcher.watched(), [root.path().to_path_buf()]);

    // A burst of writes, as an editor produces on save.
    for i in 0..5 {
        write_script(root.path(), "a", &format!("fn search(q) {{ {i}; [] }}"));
    }
    let changed = tokio::time::timeout(Duration::from_secs(5), watcher.changed())
        .await
        .expect("an event arrives");
    assert!(changed);
    assert!(
        tokio::time::timeout(Duration::from_millis(400), watcher.changed())
            .await
            .is_err(),
        "the burst was coalesced into one change"
    );
}
