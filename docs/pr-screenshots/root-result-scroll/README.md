# Long root-result evidence

The `*-wgpu.png` files render the real launcher widget tree with 40 indexed
desktop applications in an 800×320 logical-pixel viewport, at 2× scale.
Start/scrolled captures bracket a native mouse-wheel event. The query field
does not move. These are headless Iced/wgpu images, not GNOME screenshots.

The wheel test failed before the fix: application 39 remained offscreen.
Keyboard tests drive real runtime operations through the full list, wrapping
and narrowing/restoring the query, checking the whole selected row is visible
in all four presets. Selection reveal uses measured geometry, shared with the
action panel, not an estimated row offset.

Capture fresh native evidence by setting an absolute, new output directory:

```sh
COMPASS_UI_SCREENSHOT_DIR=/tmp/new-root-captures \
  cargo test -j 1 --locked -p compass-ui --lib long_root_results_scroll
```

The optional capture compares existing files if present. Ordinary tests do
not compare these committed screenshots.

`browser-dark-last.png` is the separate Chromium HTML/CSS surrogate with the
last row selected. Regenerate with `make design-states` and
`node tools/design/shoot.mjs`. The new long-list fixtures use the real root-list
model, and browser assertions reject a selected row outside the visible list.
The inspected light-start/dark-last browser frames and light-start/dark-scrolled
native frames keep the query visible and show the expected rows. Partially
visible intermediate rows are clipped by the scroll viewport. The surrogate
does not establish native input or compositor behaviour.
