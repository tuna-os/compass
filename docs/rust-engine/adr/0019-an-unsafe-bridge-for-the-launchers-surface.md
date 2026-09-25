# ADR-0019: One `unsafe` bridge to the launcher's own surface, in a crate of its own

**Status:** Accepted · **Date:** 2026-09-25 · Relates to: ADR-0014 (the first `unsafe` exception), ADR-0015 (the window is resident), `PARITY.md` "The window-material pass"

## Context

The workspace sets `unsafe_code = "forbid"`. Two crates already opt out, each for one reason stated
in its manifest: `compass-sqlcipher-sys` (registering the tokenizer on a raw `sqlite3*`, ADR-0014)
and `compass-wayland-protocols` (generated interface tables).

Background blur behind the launcher (`src/services/window-material`) was the last amber cell of the
parity ledger. The protocol client was ported (`compass_wayland::material`), but it has to name the
launcher's `wl_surface` in `get_background_effect`, and that surface is made by the toolkit on the
toolkit's connection. winit (the `xdg_toplevel` presentation) creates its connection itself, takes
none from us, and hands the surface out only as raw `wl_display*` and `wl_surface*` pointers through
`raw-window-handle`. The shortcut inhibitor avoided this under `iced_layershell` by sharing a
connection and learning the surface from `wl_keyboard.enter`; winit leaves no such opening.

Turning the pointers into proxies is `wayland-backend`'s `Backend::from_foreign_display` and
`ObjectId::from_ptr`, both `unsafe`. No maintained crate offers a safe version (`CRATE-AUDIT.md`,
"Window material"): the bridge is inherently a promise about another library's pointers.

## Decision

**The maintainer approved one more exception**, shaped like the first:

- A dedicated crate, `compass-wayland-foreign`, that does not inherit the workspace lints. It sets
  `unsafe_code = "deny"` (not `allow`), restates every other workspace lint, and explains the
  exception at the top of its manifest. One function, `adopt`, carries `#[allow(unsafe_code)]` and
  holds exactly two `unsafe` blocks; its integration test forges the handles a toolkit lends, which
  is the only other `unsafe` (test code, also allowed by name).
- Its API is safe: `bridge(&window)` takes one object that lends both handles
  (`HasWindowHandle + HasDisplayHandle`) and returns a `Connection` over the display and a
  `WlSurface`. Every invariant is either checked or made structural:
  - the pointers are read inside the toolkit's borrow of them, from one window, so the surface is
    one of the display's;
  - only the `Wayland` handle variants are accepted, and the surface's interface is checked by name;
  - a surface that is not a `wayland-rs` proxy (no liveness flag its owner clears on destruction)
    is refused, so the id is never used after the toolkit frees it;
  - the `client_system` backend is named in the manifest and called through `wayland_backend::sys`,
    so the pure-Rust backend fails to compile rather than bridging wrongly; missing libwayland is an
    error, not a panic;
  - one `Connection` per display is kept for the process's life, so its `Drop` (which touches the
    display) never runs.
- **The one invariant the types cannot express** is that winit's display outlives our connection.
  It does because the launcher window is resident (ADR-0015): winit's event loop owns the display on
  the main thread until the process ends, and the connection is used only inside
  `iced::window::run`, while the window it came from is open.
- The trait (`compass_platform::WindowMaterial`) stays in the seam crate, the implementation in the
  `vicinae` binary, and `compass-ui` never depends on the bridge (`the_seam_holds`).

## Consequences

- The `xdg_toplevel` launcher (KDE, GNOME, `VICINAE_LAYER_SHELL=0`) is blurred where the compositor
  offers `ext-background-effect-v1` (KWin; Mutter does not). Real blur is verified in the VM tier;
  headless Sway and an in-process fake compositor verify the bridge and the protocol traffic.
- The layer-shell launcher is not blurred yet: `iced_layershell` 0.19 drops `window::run`, so it
  lends no handles. The safe route there is the inhibitor's (a shared connection and
  `wl_keyboard.enter`), not this bridge.
- Two crates' worth of `unsafe` review is now three. Anything else that wants a raw toolkit handle
  goes through `compass-wayland-foreign` rather than growing a second bridge.

## What would change this

A toolkit that accepts our connection (winit taking a `wayland_client::Connection`, as
`iced_layershell` does), or a `wayland-client` API that adopts a foreign surface safely, would let
this crate go, and the workspace return to two exceptions.
