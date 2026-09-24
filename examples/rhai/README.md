# Rhai example extensions

Four copy-paste starters for the Rhai tier (PLAN.md §2.2, ADR-0005). Each is a
forty-line `.rhai` file dropped in a folder — no Node, no bundler, filterable
list with actions, capability-based sandbox by construction.

| script | `metadata().capabilities` | what it shows |
|---|---|---|
| `hello-world.rhai` | `[]` | minimal `metadata()` + `search(query)` |
| `calculator.rhai` | `[]` | pure computation (`eval`) — no I/O |
| `emoji-search.rhai` | `[]` | static array, filtering, `keywords` |
| `file-search.rhai` | `[]` (mock) | where `fs` capability would plug in |

## Authoring

```rhai
fn metadata() {
    #{ title: "My Script", icon: "star", mode: "list", capabilities: [] }
    // capabilities: [] means no host functions beyond `log`. Add "net" to get `http_get`, etc.
}

fn search(query) {
    // return a map, an array of maps, or a JSON-serialisable value — the host
    // normalises it to a ViewTree (list/grid/detail/form) for the front end.
    #{ title: "Hello " + query, subtitle: "Rhai tier" }
}
```

* The host creates the engine with `Engine::new_raw()` + `DummyModuleResolver` — `import` cannot read the filesystem.
* Limits are explicit: `set_max_operations(200_000)`, `set_max_call_levels(64)`, `set_max_string_size(1MiB)`, `set_max_array_size(10_000)`, `on_progress` budget termination.
* Every `search` runs on `tokio::task::spawn_blocking` with a 5s wall-clock timeout — never the render thread.
* Discover: `compass_script::discover(dir)` finds `.rhai` files and reads `metadata()`.

## Running

Place scripts in `~/.local/share/compass/scripts/` (or any dir passed to `discover`) and reload. See `crates/compass-script/src/lib.rs` for the capability gate — a script that did not declare `net` has no `http_get` to call.

## Shared seam

`compass-extension-api` is the front-end-agnostic seam — capability registry, view tree, action dispatch — with no knowledge of Node, JSON-RPC or Rhai (it depends only on `serde`/`tracing`). Both the TypeScript worker and Rhai scripts render through it; the test `shared_seam_rhai_and_manual_list_produce_same_view` in `crates/compass-script` proves the same fixture produces the same `ViewTree` via either path.

An empty tier is worse than no tier — these four must remain good enough to copy.
