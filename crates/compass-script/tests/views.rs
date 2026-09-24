//! What scripts return becomes the seam's `View`, and actions go back through
//! the seam's dispatch types.

mod common;

use common::{fire, fire_handler, load, runtime, titles};
use compass_extension_api::{
    ActionEffect, ActionIndex, ActionResponse, ActionStyle, DispatchError, Image, KeyModifier,
    MetadataItem, Shortcut, ToastStyle, View,
};
use compass_script::ScriptError;

#[test]
fn a_bare_array_is_a_list_with_one_untitled_section() {
    let fixture = load(
        r#"fn search(q) { [
            #{ id: "a", title: "Alpha", subtitle: q, icon: "star", keywords: ["first"],
               accessories: ["1", #{ tag: "new", color: "green" }] },
            #{ title: 42 },
        ] }"#,
        &[],
    );
    let tree = runtime().block_on(fixture.instance.search("sub")).unwrap();
    let View::List(list) = tree.root() else {
        panic!()
    };
    assert_eq!(list.sections.len(), 1);
    assert_eq!(list.sections[0].title, None);
    let alpha = &list.sections[0].items[0];
    assert_eq!(alpha.key.as_deref(), Some("a"));
    assert_eq!(alpha.subtitle.as_deref(), Some("sub"));
    assert_eq!(alpha.icon, Some(Image::builtin("star")));
    assert_eq!(alpha.keywords, ["first"]);
    assert_eq!(alpha.accessories[0].text.as_deref(), Some("1"));
    assert_eq!(alpha.accessories[1].tag.as_deref(), Some("new"));
    assert_eq!(list.sections[0].items[1].title, "42", "numbers are text");
    assert!(!list.search.host_filtering, "the script got the query");
}

#[test]
fn a_map_describes_sections_search_bar_empty_state_and_filtering() {
    let fixture = load(
        r#"fn search(q) { #{
            title: "Nav", placeholder: "Find", filtering: true, loading: true,
            sections: [#{ title: "One", subtitle: "s", items: [#{ title: "x" }] }],
            empty: #{ title: "Nothing", description: "d", icon: "ghost" },
        } }"#,
        &[],
    );
    let tree = runtime().block_on(fixture.instance.search("")).unwrap();
    let View::List(list) = tree.root() else {
        panic!()
    };
    assert_eq!(list.navigation_title.as_deref(), Some("Nav"));
    assert_eq!(list.search.placeholder.as_deref(), Some("Find"));
    assert!(list.search.host_filtering);
    assert!(list.is_loading);
    assert_eq!(list.sections[0].title.as_deref(), Some("One"));
    let empty = list.empty_state.as_ref().unwrap();
    assert_eq!(empty.title, "Nothing");
    assert_eq!(empty.icon, Some(Image::builtin("ghost")));
}

#[test]
fn markdown_at_the_top_is_a_detail_view_and_on_an_item_a_detail_pane() {
    let fixture = load(
        r##"fn search(q) {
            if q == "d" { return #{ markdown: "# Hi", metadata: [#{ title: "k", text: 1 }, ()] }; }
            [#{ title: "x", detail: #{ markdown: "body", metadata: [#{ title: "a", text: "b" }] } }]
        }"##,
        &[],
    );
    let rt = runtime();
    let tree = rt.block_on(fixture.instance.search("d")).unwrap();
    let View::Detail(detail) = tree.root() else {
        panic!()
    };
    assert_eq!(detail.markdown.as_deref(), Some("# Hi"));
    assert!(matches!(
        detail.metadata[..],
        [MetadataItem::Label { .. }, MetadataItem::Separator]
    ));

    let tree = rt.block_on(fixture.instance.search("")).unwrap();
    let View::List(list) = tree.root() else {
        panic!()
    };
    assert!(list.show_detail, "an item with a detail turns the pane on");
    assert_eq!(
        list.sections[0].items[0]
            .detail
            .as_ref()
            .unwrap()
            .markdown
            .as_deref(),
        Some("body")
    );
}

#[test]
fn a_malformed_view_names_the_path_to_the_problem() {
    let rt = runtime();
    for (source, path) in [
        (
            r#"[#{ title: "ok" }, #{ subtitle: "no title" }]"#,
            "items[1].title",
        ),
        (r#"[#{ title: "x", colour: "red" }]"#, "items[0].colour"),
        (
            r#"[#{ title: "x", actions: [#{ title: "a" }] }]"#,
            "items[0].actions[0]",
        ),
        (
            r#"[#{ title: "x", actions: [#{ title: "a", run: "nope" }] }]"#,
            "items[0].actions[0].run",
        ),
        (
            r#"[#{ title: "x", actions: [#{ title: "a", run: "f", shortcut: "hyper+k" }] }]"#,
            "items[0].actions[0].shortcut",
        ),
        (r#"#{ sections: [42] }"#, "sections[0]"),
        ("42", "the returned value"),
    ] {
        let fixture = load(&format!("fn f() {{}} fn search(q) {{ {source} }}"), &[]);
        let error = rt.block_on(fixture.instance.search("")).unwrap_err();
        let ScriptError::InvalidView { path: got, .. } = &error else {
            panic!("{source}: {error:?}");
        };
        assert_eq!(got, path, "{source}: {error}");
    }
}

#[test]
fn actions_carry_shortcuts_styles_and_ids_and_run_named_functions_with_args() {
    let fixture = load(
        r#"
        fn done(args) { #{ toast: `did ${args.n}`, style: "success", message: "m", rerender: true } }
        fn close() { #{ hud: "bye" } }
        fn search(q) { [#{ title: "row", actions: [
            #{ id: "d", title: "Do", run: "done", args: #{ n: 7 }, shortcut: "ctrl+shift+d",
               style: "destructive", icon: "bolt" },
            #{ title: "Close", run: "close" },
        ] }] }"#,
        &[],
    );
    let rt = runtime();
    let tree = rt.block_on(fixture.instance.search("")).unwrap();
    let View::List(list) = tree.root() else {
        panic!()
    };
    let panel = list.sections[0].items[0].actions.as_ref().unwrap();
    let action = panel.actions()[0];
    assert_eq!(action.key.as_deref(), Some("d"));
    assert_eq!(action.style, ActionStyle::Destructive);
    assert_eq!(
        action.shortcut,
        Some(Shortcut::new([KeyModifier::Ctrl, KeyModifier::Shift], "d"))
    );
    assert_eq!(action.icon, Some(Image::builtin("bolt")));

    let ActionResponse::Completed { effects, .. } = rt.block_on(fire(&fixture, &tree, "Do")) else {
        panic!()
    };
    assert_eq!(
        effects,
        [
            ActionEffect::Toast {
                style: ToastStyle::Success,
                title: "did 7".to_owned(),
                message: Some("m".to_owned()),
            },
            ActionEffect::Rerender,
        ]
    );
    let ActionResponse::Completed { effects, .. } = rt.block_on(fire(&fixture, &tree, "Close"))
    else {
        panic!()
    };
    assert_eq!(
        effects,
        [ActionEffect::Hud {
            text: "bye".to_owned()
        }]
    );
}

#[test]
fn closures_capture_the_row_they_were_made_for() {
    let fixture = load(
        r#"fn search(q) {
            let rows = [];
            for name in ["a", "b"] {
                rows.push(#{ title: name, actions: [#{ title: `Pick ${name}`, run: || #{ hud: name } }] });
            }
            rows
        }"#,
        &[],
    );
    let rt = runtime();
    let tree = rt.block_on(fixture.instance.search("")).unwrap();
    let ActionResponse::Completed { effects, .. } = rt.block_on(fire(&fixture, &tree, "Pick b"))
    else {
        panic!()
    };
    assert_eq!(
        effects,
        [ActionEffect::Hud {
            text: "b".to_owned()
        }]
    );
}

#[test]
fn an_action_from_an_older_render_is_an_unknown_action() {
    let fixture = load(
        r#"fn search(q) { [#{ title: q, actions: [#{ title: "Go", run: || () }] }] }"#,
        &[],
    );
    let rt = runtime();
    let old = rt.block_on(fixture.instance.search("old")).unwrap();
    let old_index = ActionIndex::from_tree(&old);
    let old_handler = old.actions()[0].handler.clone();
    let new = rt.block_on(fixture.instance.search("new")).unwrap();
    assert_eq!(titles(&new), ["new"]);
    assert_ne!(
        new.actions()[0].handler,
        old_handler,
        "tokens are not reused"
    );

    let response = rt.block_on(fire_handler(&fixture, &old_index, &old_handler));
    assert!(
        matches!(
            response,
            ActionResponse::Failed {
                error: DispatchError::UnknownAction { .. },
                ..
            }
        ),
        "{response:?}"
    );
}

#[test]
fn a_failing_action_is_a_failed_response_naming_the_script() {
    let fixture = load(
        r#"fn boom() { throw "kaboom" } fn search(q) { [#{ title: "x", actions: [#{ title: "Boom", run: "boom" }] }] }"#,
        &[],
    );
    let rt = runtime();
    let tree = rt.block_on(fixture.instance.search("")).unwrap();
    let response = rt.block_on(fire(&fixture, &tree, "Boom"));
    let ActionResponse::Failed {
        error: DispatchError::Failed { extension, message },
        ..
    } = response
    else {
        panic!("{response:?}")
    };
    assert_eq!(extension, fixture.instance.manifest().id);
    assert!(message.contains("kaboom"), "{message}");
}

#[test]
fn identical_output_yields_identical_trees() {
    // Node ids are derived, so the launcher can diff instead of rebuild.
    let fixture = load(
        r#"fn search(q) { [#{ id: "k", title: "x" }, #{ title: "y" }] }"#,
        &[],
    );
    let rt = runtime();
    let a = rt.block_on(fixture.instance.search("")).unwrap();
    let b = rt.block_on(fixture.instance.search("")).unwrap();
    assert_eq!(a.nodes(), b.nodes());
    assert!(compass_extension_api::ViewDiff::between(&a, &b).is_empty());
}
