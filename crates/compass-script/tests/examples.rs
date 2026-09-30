//! The first-party examples in `extensions/rhai-examples/` are part of the
//! product: the tier ships only while they are good enough to copy (PLAN
//! Phase 5, Track C). These tests keep them working and pin what each one
//! demonstrates.

mod common;

use std::path::PathBuf;
use std::sync::Arc;

use common::{Fixture, fire, runtime, titles};
use compass_extension_api::{ActionEffect, ActionResponse, CapabilityRegistry, View};
use compass_script::{HostCall, Limits, MemoryHost, ScriptInstance, discovery};

fn examples_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../extensions/rhai-examples")
}

/// Loads one example with everything it declares granted.
fn example(name: &str) -> Fixture {
    let script = discovery::load(&examples_dir().join(name)).expect("example loads");
    let mut registry = CapabilityRegistry::new();
    script.manifest.declare_into(&mut registry);
    for cap in &script.manifest.capabilities {
        registry.grant(&script.manifest.id, cap).expect("grantable");
    }
    let host = Arc::new(MemoryHost::new());
    let instance = ScriptInstance::load(&script, &registry, host.clone(), Limits::default())
        .unwrap_or_else(|error| panic!("{name}: {error}"));
    Fixture {
        instance,
        host,
        registry,
    }
}

fn completed(response: &ActionResponse) -> &[ActionEffect] {
    match response {
        ActionResponse::Completed { effects, .. } => effects,
        ActionResponse::Failed { error, .. } => panic!("action failed: {error}"),
    }
}

#[test]
fn at_least_four_examples_ship_and_every_one_loads_with_what_it_declares() {
    let scan = discovery::scan(&[examples_dir()]);
    assert!(scan.failed.is_empty(), "{:?}", scan.failed);
    assert!(scan.scripts.len() >= 4, "found {}", scan.scripts.len());
    let rt = runtime();
    for script in &scan.scripts {
        let name = script
            .manifest
            .directory
            .file_name()
            .unwrap()
            .to_string_lossy();
        assert!(script.manifest.icon.is_some(), "{name} has no icon");
        assert!(
            script.manifest.description.is_some(),
            "{name} has no description"
        );
        assert!(
            script.source.lines().take(3).any(|l| l.starts_with("//")),
            "{name} does not open with a comment saying what it shows"
        );
        let fixture = example(&name);
        for query in ["", "a", "10", "uuid 2", "lorem", "10 km"] {
            rt.block_on(fixture.instance.search(query))
                .unwrap_or_else(|error| panic!("{name} on {query:?}: {error}"));
        }
    }
}

#[test]
fn unit_converter_puts_the_asked_for_unit_first_and_copies_it() {
    let fixture = example("unit-converter");
    let rt = runtime();
    let tree = rt.block_on(fixture.instance.search("10 km to mi")).unwrap();
    let rows = titles(&tree);
    assert_eq!(rows[0], "6.2137 mi");
    assert!(rows.contains(&"10000 m".to_owned()), "{rows:?}");
    assert!(
        !rows.iter().any(|r| r.ends_with(" kg")),
        "only lengths: {rows:?}"
    );

    let response = rt.block_on(fire(&fixture, &tree, "Copy Result"));
    assert_eq!(completed(&response), [ActionEffect::CloseWindow]);
    assert_eq!(fixture.host.clipboard().as_deref(), Some("6.2137"));

    let tree = rt.block_on(fixture.instance.search("212f in c")).unwrap();
    assert_eq!(titles(&tree)[0], "100 c");
    let tree = rt.block_on(fixture.instance.search("hello")).unwrap();
    let View::List(list) = tree.root() else {
        panic!()
    };
    assert!(list.empty_state.is_some());
}

#[test]
fn epoch_converter_goes_both_ways_and_reads_milliseconds() {
    let fixture = example("epoch-converter");
    let rt = runtime();
    let tree = rt.block_on(fixture.instance.search("1700000000")).unwrap();
    assert_eq!(titles(&tree)[0], "2023-11-14T22:13:20Z");
    let tree = rt
        .block_on(fixture.instance.search("1700000000000"))
        .unwrap();
    assert_eq!(titles(&tree)[0], "2023-11-14T22:13:20Z");
    let tree = rt
        .block_on(fixture.instance.search(" 2023-11-14T22:13:20Z "))
        .unwrap();
    assert!(titles(&tree).contains(&"1700000000".to_owned()));
    let tree = rt.block_on(fixture.instance.search("2024-03-01")).unwrap();
    assert!(titles(&tree).contains(&"1709251200".to_owned()));
    assert!(
        titles(&tree)
            .iter()
            .any(|t| t.starts_with("Friday, 01 March 2024")),
        "{:?}",
        titles(&tree)
    );
    let tree = rt.block_on(fixture.instance.search("not a date")).unwrap();
    let View::List(list) = tree.root() else {
        panic!()
    };
    assert!(list.empty_state.is_some());
}

#[test]
fn generators_make_what_was_asked_for_and_paste_it() {
    let fixture = example("generators");
    let rt = runtime();
    let tree = rt.block_on(fixture.instance.search("uuid 5")).unwrap();
    let ids = titles(&tree);
    assert_eq!(ids.len(), 5);
    for id in &ids {
        assert_eq!(id.len(), 36);
        assert_eq!(&id[14..15], "4", "version 4: {id}");
    }
    let tree = rt.block_on(fixture.instance.search("password 32")).unwrap();
    assert_eq!(titles(&tree)[0].chars().count(), 32);
    let response = rt.block_on(fire(&fixture, &tree, "Paste"));
    assert_eq!(completed(&response), [ActionEffect::CloseWindow]);
    assert!(matches!(&fixture.host.calls()[..], [HostCall::Paste(p)] if p.chars().count() == 32));

    let tree = rt.block_on(fixture.instance.search("lorem 2")).unwrap();
    let View::List(list) = tree.root() else {
        panic!()
    };
    assert!(list.show_detail);
    let text = list.sections[0].items[0]
        .detail
        .as_ref()
        .and_then(|d| d.markdown.clone())
        .unwrap();
    assert_eq!(text.split("\n\n").count(), 2);
}

#[test]
fn quick_notes_adds_filters_and_deletes_through_storage() {
    let fixture = example("quick-notes");
    let rt = runtime();
    let id = fixture.instance.manifest().id.clone();

    let tree = rt.block_on(fixture.instance.search("")).unwrap();
    assert!(titles(&tree).is_empty());

    for text in ["buy milk", "call Ada"] {
        let tree = rt.block_on(fixture.instance.search(text)).unwrap();
        let response = rt.block_on(fire(&fixture, &tree, "Add Note"));
        assert!(completed(&response).contains(&ActionEffect::Rerender));
    }
    assert!(fixture.host.storage(&id).contains_key("notes"));

    let tree = rt.block_on(fixture.instance.search("")).unwrap();
    assert_eq!(titles(&tree), ["call Ada", "buy milk"], "newest first");
    let tree = rt.block_on(fixture.instance.search("milk")).unwrap();
    assert_eq!(titles(&tree), ["Add note: milk", "buy milk"]);

    let response = rt.block_on(fire(&fixture, &tree, "Delete Note"));
    assert!(completed(&response).contains(&ActionEffect::Rerender));
    let tree = rt.block_on(fixture.instance.search("")).unwrap();
    assert_eq!(titles(&tree), ["call Ada"]);
}

#[test]
fn web_search_opens_an_encoded_url() {
    let fixture = example("web-search");
    let rt = runtime();
    let tree = rt.block_on(fixture.instance.search("rust & rhai")).unwrap();
    let response = rt.block_on(fire(&fixture, &tree, "Open in Browser"));
    assert_eq!(completed(&response), [ActionEffect::CloseWindow]);
    assert_eq!(
        fixture.host.calls(),
        [HostCall::Open(
            "https://duckduckgo.com/?q=rust%20%26%20rhai".to_owned()
        )]
    );
    let tree = rt.block_on(fixture.instance.search("")).unwrap();
    assert!(tree.actions().is_empty(), "nothing to open without a query");
}
