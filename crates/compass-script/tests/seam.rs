//! Shared-seam test (PLAN §8.2): one fixture, expressed once as what the
//! TypeScript reconciler sends and once as a Rhai script, must reach the
//! launcher as the same `View` through `compass-extension-api`.
//!
//! This is the regression test that keeps the tiers from drifting. The only
//! thing allowed to differ is the handler tokens, which each tier mints its
//! own way and the host treats as opaque.

mod common;

use common::{load, runtime};
use compass_extension_api::{View, ViewTree};
use compass_worker_host::render::RenderNode;
use compass_worker_host::view_model::to_view;
use serde_json::{Value, json};

const SCRIPT: &str = r##"
fn search(q) {
    #{
        title: "Repos",
        placeholder: "Filter",
        filtering: true,
        show_detail: true,
        sections: [
            #{ items: [#{ id: "l", title: "loose" }] },
            #{ title: "Mine", items: [#{
                id: "c",
                title: "compass",
                subtitle: "tuna-os",
                icon: "star",
                keywords: ["launcher"],
                accessories: ["12", #{ tag: "rust" }],
                detail: "# compass",
                actions: [
                    #{ title: "Copy URL", run: || () },
                    #{ id: "open", title: "Open", run: || (), shortcut: "ctrl+shift+o" },
                ],
            }] },
        ],
    }
}
"##;

fn typescript() -> View {
    let root: RenderNode = serde_json::from_value(json!({
        "$t": "list",
        "navigationTitle": "Repos",
        "searchBarPlaceholder": "Filter",
        "isShowingDetail": true,
        "children": [
            {"$t": "list-item", "title": "loose", "id": "l"},
            {"$t": "list-section", "title": "Mine", "children": [
                {"$t": "list-item", "title": "compass", "subtitle": "tuna-os", "id": "c",
                 "keywords": ["launcher"], "icon": {"source": {"raw": "star"}},
                 "accessories": [{"text": "12"}, {"tag": {"value": "rust"}}],
                 "children": [
                    {"$t": "action-panel", "children": [
                        {"$t": "action", "title": "Copy URL", "onAction": "cb-7"},
                        {"$t": "action", "title": "Open", "onAction": "cb-8", "stableId": "open",
                         "shortcut": {"key": "o", "modifiers": ["ctrl", "shift"]}}
                    ]},
                    {"$t": "list-item-detail", "markdown": "# compass"}
                 ]}
            ]}
        ]
    }))
    .expect("a render node");
    to_view(&root).expect("a list")
}

/// The tree as JSON, with every handler token replaced by one placeholder.
fn normalised(tree: &ViewTree) -> Value {
    fn walk(value: &mut Value) {
        match value {
            Value::Object(map) => {
                for (key, child) in map.iter_mut() {
                    if key == "handler" {
                        *child = Value::String("<handler>".to_owned());
                    } else {
                        walk(child);
                    }
                }
            }
            Value::Array(items) => items.iter_mut().for_each(walk),
            _ => {}
        }
    }
    let mut value = serde_json::to_value(tree).expect("serialises");
    walk(&mut value);
    value
}

#[test]
fn the_same_fixture_from_either_tier_is_the_same_view() {
    let from_typescript = ViewTree::new(typescript());
    let fixture = load(SCRIPT, &[]);
    let from_rhai = runtime().block_on(fixture.instance.search("")).unwrap();

    assert_eq!(normalised(&from_rhai), normalised(&from_typescript));
    // Identity is derived from structure and keys, not tokens, so the node
    // ids agree too: the launcher cannot tell which tier drew the list.
    assert_eq!(from_rhai.nodes().len(), from_typescript.nodes().len());
    let ids = |t: &ViewTree| t.nodes().into_iter().map(|n| n.id).collect::<Vec<_>>();
    assert_eq!(ids(&from_rhai), ids(&from_typescript));
}
