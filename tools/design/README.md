# The design surrogate

A browser rendering of the launcher, so a design change can be looked at in a
second instead of after a twenty-minute VM tier run.

```sh
make design         # regenerate states.json and serve the page
make design-shots   # regenerate and screenshot every state, both appearances
```

## What makes it not a mock

Two things, and they are the whole point:

- **The states come from the real view models.** `states.json` is printed by
  `cargo run -p compass-ui --example design_states`, which calls
  `root_list::build` and `action_panel::flatten` — the same functions the Iced
  app calls. The sections, the ordering, the flattened panel rows and which row
  the selection lands on are the ported logic's answers, not a designer's guess
  at them.
- **The tokens come from the Rust.** Every colour and measurement is read from
  `crates/compass-ui/src/design.rs` through that same JSON. Nothing is retyped
  into the CSS; the stylesheet only has custom properties, set at load. Change
  a token in Rust and the page moves with it.

It found a bug within a minute of first rendering: the fixture built items with
`RootItemMeta::default()`, which leaves `enabled` false, so the search dropped
every one and a query with three obvious matches showed "No results".

## What it does not prove

**That Iced paints this.** The surrogate is HTML and CSS; the real launcher is
wgpu through Iced's widget tree. Layout, colour, hierarchy and state are worth
judging here. Whether the real renderer produces the same picture is the VM
tier's question, and it remains the only thing that answers it.

Two smaller gaps, both visible on the page:

- **Icons are stand-ins.** `RootItem` carries no icon; the real launcher
  resolves one through the XDG icon theme, which a browser has no access to.
  The page draws the first letter at the right size, so row proportions can be
  judged against something.
- **Text metrics are approximate.** The page is given the same font stack, but
  a browser and wgpu do not lay out glyphs identically.

The action filter is a focused, read-only preview of the native input. Select
`panel`, `panel-filtered` or `panel-no-actions` to inspect the Rust-filtered
rows; typing into the browser does not drive the application. The screenshot
runner checks its value, focus, absence of a second root-query caret, and empty
action notice before capturing each panel state. It also checks the filter is
inside the visible panel/card: focus alone does not prove it is visible. The
preview panel scrolls when its fixture actions exceed the card height; native
scrolling and keyboard selection still require native tests. Fixture-only management
actions do not establish that those actions are wired in the launcher.

See `../../docs/rust-engine/RENDER-HARNESSES.md` for how this fits beside the
other two harnesses, and for the Playwright mechanics that bite.

## A higher-fidelity option, not taken yet

Iced 0.14 builds for `wasm32` — `iced_winit` carries `web-sys` and
`HtmlCanvasElement` dependencies and `iced` has a `webgl` feature routing to
`iced_wgpu/webgl`. That would put the *real* widget code and the *real* renderer
in a canvas, which this cannot. What stands in the way is that `compass-ui`
depends on `compass-core`, `compass-xdg` and `compass-platform`: filesystem
scanning, D-Bus and process spawning, none of which compiles to `wasm32`. It
needs a small entry point that feeds a fixed in-memory index and stubs the
launch. Worth doing; it is a different job from this one, and this one is what
makes a design iteration cheap.

**`iced_web` is not that option.** It is archived, targets the Iced 0.1/0.2 API,
and renders to the DOM through dodrio rather than through wgpu — so reviving it
would be both a large port and a *different renderer from the one that ships*,
which is the one thing a fidelity harness must not be.
