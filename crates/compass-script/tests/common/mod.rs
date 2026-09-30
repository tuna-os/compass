//! Helpers shared by the integration tests.

#![allow(dead_code)]

use std::path::Path;
use std::sync::Arc;

use compass_extension_api::{
    ActionIndex, ActionPayload, ActionRequest, ActionResponse, Capability, CapabilityRegistry,
    HandlerId, InvocationSource, Pending, ViewTree,
};
use compass_script::{Limits, MemoryHost, ScriptError, ScriptInstance, ScriptManifest};

/// A script built from source, with `declared` in its manifest and `granted`
/// granted by the host.
pub struct Fixture {
    pub instance: ScriptInstance,
    pub host: Arc<MemoryHost>,
    pub registry: CapabilityRegistry,
}

pub fn manifest(declared: &[&str]) -> ScriptManifest {
    let caps = declared
        .iter()
        .map(|c| format!("\"{c}\""))
        .collect::<Vec<_>>()
        .join(", ");
    ScriptManifest::parse(
        &format!("title = \"Test\"\ncapabilities = [{caps}]"),
        Path::new("/scripts/test"),
    )
    .expect("manifest")
}

pub fn registry(manifest: &ScriptManifest, granted: &[&str]) -> CapabilityRegistry {
    let mut registry = CapabilityRegistry::new();
    manifest.declare_into(&mut registry);
    for cap in granted {
        registry
            .grant(&manifest.id, &Capability::new(*cap))
            .expect("grant");
    }
    registry
}

pub fn try_load(
    source: &str,
    declared: &[&str],
    granted: &[&str],
    limits: Limits,
) -> Result<Fixture, ScriptError> {
    let manifest = manifest(declared);
    let registry = registry(&manifest, granted);
    let host = Arc::new(MemoryHost::new());
    let instance = ScriptInstance::compile(manifest, source, &registry, host.clone(), limits)?;
    Ok(Fixture {
        instance,
        host,
        registry,
    })
}

pub fn load(source: &str, caps: &[&str]) -> Fixture {
    try_load(source, caps, caps, Limits::default()).expect("script compiles")
}

pub fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("runtime")
}

/// Fires the action titled `title` in `tree` through the seam's own index and
/// pending tracker, the way a host does.
pub async fn fire(fixture: &Fixture, tree: &ViewTree, title: &str) -> ActionResponse {
    let index = ActionIndex::from_tree(tree);
    let handler = tree
        .actions()
        .into_iter()
        .find(|a| a.title == title)
        .unwrap_or_else(|| panic!("no action titled {title}"))
        .handler;
    fire_handler(fixture, &index, &handler).await
}

pub async fn fire_handler(
    fixture: &Fixture,
    index: &ActionIndex,
    handler: &HandlerId,
) -> ActionResponse {
    let mut pending = Pending::new();
    let request: ActionRequest = pending
        .begin(
            index,
            &fixture.instance.manifest().id,
            handler,
            InvocationSource::Panel,
            ActionPayload::None,
        )
        .expect("known action");
    let response = fixture.instance.invoke(&request).await;
    pending.complete(&response).expect("answers the invocation");
    response
}

/// Every row title in a list tree, in order.
pub fn titles(tree: &ViewTree) -> Vec<String> {
    match tree.root() {
        compass_extension_api::View::List(list) => list
            .sections
            .iter()
            .flat_map(|s| &s.items)
            .map(|i| i.title.clone())
            .collect(),
        other => panic!("not a list: {other:?}"),
    }
}
