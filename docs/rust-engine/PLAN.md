# Compass Rust Engine — Transformation Plan

**Status:** proposal, not yet approved for implementation
**First target:** **GNOME 50 / 51 on [Bluefin](https://projectbluefin.io/)** — everything below is
sequenced around that, with other compositors as later phases.
**Scope:** replace the C++23/Qt6 core of this repository with a Rust workspace, seeded from
[MystikoLab/rustcast](https://github.com/MystikoLab/rustcast), following the Rust engine
specification in [this gist](https://gist.github.com/hanthor/ba051ebc406ddb4b8f958601378a5234)
(the spec is in the gist **comment**; the gist body is the earlier C++/Qt6 variant and is treated
here as reference).

Companion document: [`REFERENCES.md`](./REFERENCES.md) — prior art, verified crate versions, and
the protocol-support evidence behind §3.

---

## 0. Decisions taken

| Decision | Choice | Reversibility |
|---|---|---|
| Direction | Full Rust rewrite of the core, seeded from rustcast | Hard — this is the plan |
| Host repo | `tuna-os/compass`, Rust workspace alongside the C++ tree | Easy |
| **First target** | **GNOME 50/51 on Bluefin, shipped as a Flatpak** | Hard — reshapes phases 1–3 |
| UI toolkit | **Iced 0.14** (rustcast already ships a working Iced launcher) over Slint | Medium — contained in `compass-ui` |
| Surface strategy | plain `xdg_toplevel` first (**GNOME has no layer-shell**); `wlr-layer-shell` added in Phase 5 | Easy |
| Extension runtime | Keep `src/typescript/` (Raycast-compat SDK) **unchanged**; only its host is rewritten | Easy |
| Third extension tier | **Rhai** scripts in-process, behind the same capability layer as the TS host (§2.2) | Easy — drop it if the seam doesn't materialise |
| Crate prefix | `compass-*`, binary stays `vicinae` for CLI/config/socket compatibility | Trivial |
| Licence | Compass is GPL-3.0, rustcast is MIT; MIT → GPL-3.0 is one-way compatible, so rustcast code may be incorporated with its copyright header plus a provenance note | N/A |

Decisions are recorded as ADRs in [`adr/`](./adr/); §10 summarises them and lists what is still open.

---

## 1. Honest inventory

### 1.1 What compass is today

| Area | LOC | Notes |
|---|---|---|
| C++ (`.cpp/.hpp/.h`) | 144,534 | of which `src/lib/glyph` is 42,636 — mostly **generated** emoji/symbol datasets |
| QML | 14,660 | presentation only, per the `AGENTS.md` bridge pattern |
| TypeScript | 11,753 | `src/typescript/{api,extension-manager,raycast-api-compat}` |

Hand-written C++ to be replaced is ≈ **100k LOC**:

| Subsystem | LOC | Weight |
|---|---|---|
| `src/server/src/services/` (45 services) | 31,765 | heaviest; the actual product |
| `src/server/src/ui/` | 15,243 | view hosts feeding QML |
| `src/server/src/builtins/` (17 groups) | 11,974 | breadth, individually small |
| `src/server/src/extension/` | 6,585 | Node host + React reconciler bridge |
| `src/file-indexer/` | 5,646 | standalone daemon, already isolated |
| `src/lib/xdgpp/` | 4,235 | desktop entry / MIME / locale, **10 test files** |
| `src/lib/figura/` | 2,875 | in-tree IPC code generator |
| `src/cli/`, `src/lib/fuzzy/`, `src/data-control-server/`, `src/snippet/` | 5,450 | |
| `src/browser-extension/` | 250 | **out of scope** — becomes an extension, ADR-0008 |

Assets the rewrite inherits rather than invents: Wayland protocol XML
(`wlr-layer-shell`, `wlr-foreign-toplevel-management`, `xx-hotkey-v1`, `vicinae-hotkey-v1`);
window-manager backends for gnome/hyprland/kde/niri/wayland/x11/macos/windows; global-shortcut
backends including the `xx-hotkey-v1` one from
[#1936](https://github.com/tuna-os/compass/pull/1936); and Catch2/CTest suites under
`src/lib/{fuzzy,xdgpp,crypto,glyph,script-command}/tests`, `src/lib/vicinae-ipc/tests`,
`src/file-indexer/tests`, `src/snippet/tests`, all run by `make test`.

### 1.2 What rustcast gives us

**10,670 LOC of Rust, single crate, macOS-only, edition 2024.**

Reusable (≈3–4k LOC after de-macOS-ing): `app.rs` + `app/tile*` + `app/pages/*` (a working Iced 0.14
launcher shell), `styles.rs` (501), `config.rs` (407), `database.rs` + `migrations/`,
`unit_conversion.rs` (478), `debounce.rs`, `utils.rs`, `commands.rs` — plus dependency choices that
already match the spec (`tokio`, `nucleo-matcher`, `rusqlite`, `arboard`, `iced`).

Discarded: all of `src/platform/macos/` and every `objc2*` crate; `global-hotkey` (→ portal);
`icns`, `tray-icon`, `objc2-app-kit` screen handling; and the single-crate layout.

**Calibration:** rustcast has 5 pages; compass has 17 builtin groups and 45 services. rustcast covers
roughly **5% of compass's functional surface**. It is a seed and a proof that the Iced shell works —
not a base to bolt features onto. Schedule accordingly.

---

## 2. Target architecture

```
                        ┌──────────────────────┐
                        │  bin/vicinae (CLI)   │  ext install/remove/list, doctor, toggle
                        └───────────┬──────────┘
     ┌──────────────┬───────────────┼───────────────┬──────────────────┐
     ▼              ▼               ▼               ▼                  ▼
┌──────────┐  ┌───────────┐  ┌────────────┐  ┌─────────────┐  ┌────────────────┐
│compass-ui│  │compass-   │  │compass-    │  │compass-     │  │compass-worker- │
│  (Iced)  │  │  core     │  │  search    │  │  ipc        │  │      host      │
└────┬─────┘  │(state,    │  │(nucleo)    │  │(UDS, framed)│  │(spawn+sandbox) │
     │        │ registry, │  └────────────┘  └─────────────┘  └───────┬────────┘
     │        │ dispatch) │                                            │ JSON-RPC 2.0
     │        └─────┬─────┘                                            ▼
     ▼              ▼                                        ┌──────────────────┐
┌──────────────┬──────────────────┬───────────────┬──────────│ vicinae-worker-ts│
│compass-shell │ compass-portals  │compass-wayland│ compass- │ (Node, unchanged │
│ (GNOME Shell │ (ashpd: hotkeys, │(xdg_toplevel, │ platform │  @raycast/api)   │
│  DBus, zbus) │  OpenURI, files) │ activation,   │ (fs,exec,└──────────────────┘
│              │                  │ layer-shell*) │  icons)
└──────────────┴──────────────────┴───────────────┴──────────┘
                                     * Phase 5, wlroots only
```

| Crate | Owns | Ported from |
|---|---|---|
| `compass-core` | state machine, config, root-item registry, dispatch, frecency, SQLite | C++ `server/src/{root-search,config,command,services}` + rustcast `database.rs`, `config.rs` |
| `compass-search` | fuzzy index + matcher, `FuzzySearchable`-equivalent trait | C++ `lib/fuzzy` semantics on `nucleo` |
| `compass-shell` | **GNOME Shell DBus client** (windows, clipboard, paste) via `zbus` | C++ `services/{window-manager,clipboard}/gnome` |
| `compass-wayland` | `xdg_toplevel` surface, `xdg-activation-v1`, `keyboard-shortcuts-inhibit`; later `wlr-layer-shell` + `ext-foreign-toplevel-list-v1` | C++ `internal/wayland`, `ui/windows`, `services/window-manager/*` |
| `compass-portals` | `ashpd`: GlobalShortcuts, OpenURI, FileChooser, Screenshot, Secret | C++ `services/{global-shortcuts,file-chooser,permissions}` |
| `compass-ipc` | UDS at `$XDG_RUNTIME_DIR/vicinae/ipc.sock`, length-prefixed frames | C++ `lib/vicinae-ipc`, `lib/figura` |
| `compass-xdg` | desktop entries, MIME, icon theme, locale | C++ `lib/xdgpp` |
| `compass-extension-api` | **front-end-agnostic seam**: capability registry, view tree, action dispatch. Knows nothing about Node or Rhai | new — see §2.2 |
| `compass-worker-host` | Node worker lifecycle, Landlock + seccomp, cgroups v2, state dirs | C++ `server/src/extension/node-runtime` |
| `compass-script` | in-process [Rhai](https://rhai.rs) host: engine per script, capability-gated registration, operation budget | new — see §2.2 |
| `compass-ui` | Iced views, theming, tiles, pages | rustcast `app/*`, `styles.rs` |
| `compass-platform` | process exec (incl. `flatpak-spawn`), file indexing, clipboard | C++ `file-indexer`, `services/{clipboard,paste}` |

### 2.1 Three deliberate departures from the gist spec

(A fourth, the Rhai tier, is additive rather than a departure and is described in §2.2.)

1. **postcard, not Cap'n Proto.** The spec wants Cap'n Proto zero-copy on the core socket. We already
   have a working framed protocol and an in-tree generator (`figura`); a second wire format buys
   latency we have not measured a need for. Start with `serde` + length-prefixed
   [postcard](https://crates.io/crates/postcard) frames, keep JSON-RPC on the *worker* socket for
   `@raycast/api` compatibility, and adopt Cap'n Proto only if Phase-4 benchmarks miss the 0.5 ms
   SLA. Recorded as ADR-0002 — reversible, not rejected.
2. **Internationalisation, which the spec omits entirely.** Compass has a live Qt Linguist catalogue
   (`src/server/translations/*.ts`) and strict i18n rules in `AGENTS.md`. Proposal: `fluent-rs` plus
   a build-time extractor and a one-off `.ts` → `.ftl` converter so existing translations survive.
   Dropping the catalogue is a regression users notice immediately. ADR-0003.
3. **The GNOME Shell extension stays.** See §3 — on GNOME 50/51 there is no protocol alternative for
   window management, clipboard history, or paste. The spec's headline goal ("eliminate reliance on
   GNOME Shell private APIs") is not achievable on our first target, and pretending otherwise would
   design us into a corner. The realistic goal is restated in §3.5.

### 2.2 A third extension tier: Rhai scripts

Compass has two extensibility tiers today and they leave a gap in the middle:

| Tier | Power | Cost to the author | Cost to us |
|---|---|---|---|
| Raycast TS/React extensions | full | Node, npm, a bundler, React | a sandboxed worker process, ~256 MB ceiling, tens of ms to spawn |
| Script commands (shell, python, …) | one-shot output | trivial | arbitrary process execution, no sandbox, no interactive view |
| **← the gap →** | **interactive, stateful, sandboxed, no runtime dependency** | | |

[Rhai](https://rhai.rs) fills it. A forty-line `.rhai` file dropped in a folder gets a filterable list
view with actions, at the cost of parsing an AST (microseconds) rather than spawning Node. On an
immutable Flatpak target where Node has to be bundled, that matters for our own SLAs in §8.5.

**Rhai's sandbox is stronger than the Node one, and for a structural reason.** Rhai's standard
library is pure computation — no filesystem, no network, no process. The host registers every
capability a script can reach, so a script that did not declare `net` cannot make an HTTP call
because the function does not exist in its scope. That is capability-based security by
construction, versus the Node worker where we start from full access and subtract with Landlock and
seccomp. Verified limit APIs: `set_max_operations`, `set_max_call_levels`, `set_max_string_size`,
`set_max_array_size`, `set_max_expr_depths`, `set_max_modules`, and `on_progress` for
budget-exhaustion termination.

**One sharp edge:** `Engine::new` installs `FileModuleResolver` by default, so `import` reads
`.rhai` files off disk. Use `Engine::new_raw()` with an explicit package, or
`DummyModuleResolver`, or a resolver scoped to the script's own bundle. This must be a test, not a
code review note.

Sketch of the shape, illustrative and not settled:

```rhai
fn metadata() {
    #{ title: "Jira Issues", icon: "jira", mode: "list", capabilities: ["net"] }
}

fn search(query) {
    http::get_json(`https://example.invalid/search?q=${query}`).map(|i| #{
        title: i.summary,
        subtitle: i.key,
        actions: [ #{ title: "Open", run: || shell::open(i.url) } ],
    })
}
```

**The architectural consequence, and the reason this is written down now rather than in Phase 6.**
A third extension tier is only affordable if it is a second *front-end* onto one capability layer,
not a parallel stack. Otherwise every new host capability — clipboard read, window list, storage,
OAuth — has to be exposed three times and will drift. So `compass-extension-api` is carved out as
a seam in **Phase 4**, when we are designing the TS host's view protocol anyway, and Rhai becomes a
consumer of it in Phase 5. Getting that seam wrong is what makes this expensive; getting it right
makes Rhai mostly a binding exercise.

Two honest counterpoints, recorded so nobody is surprised later:

- **The ecosystem is zero.** The Raycast store is why people choose Vicinae. Nobody has written a
  Rhai launcher extension, and we would have to seed the tier with first-party examples and real
  docs. This is a product bet, not a technical one — see §10.9.
- **Rhai is synchronous and in-process**, so a script can hang the UI. Every script runs on
  `tokio::task::spawn_blocking`, never the render thread, with an operation budget and a wall-clock
  timeout. Host functions that do I/O block from the script thread into the runtime, which bounds
  how many can be in flight.

Alternatives considered: `mlua` (Lua — bigger ecosystem, but a C dependency and a weaker sandbox
story), `wasmtime` + WASI (strongest isolation and any source language, but a heavy lift and a poor
fit for forty-line scripts), and `rquickjs` / `boa_engine` (JS in-process — tempting since authors
already know JS, but owning a second JS runtime with a *different* API surface from the TS tier is
worse than Rhai's honest separateness). Recorded as ADR-0005.

---

## 3. Target-platform reality: GNOME 50/51

This section is the most important in the document, because it contradicts the spec. Every claim
below was checked against primary sources — see [`REFERENCES.md`](./REFERENCES.md) §3 for the
receipts.

### 3.1 What Mutter actually implements

From `src/meson.build` on `mutter` main (51.rc), the full Wayland protocol list includes
`xdg-shell`, `xdg-activation-v1`, `xdg-dialog`, `xdg-toplevel-tag`, `xdg-session-management`,
`keyboard-shortcuts-inhibit-v1`, `text-input-v3`, `fractional-scale-v1`, `cursor-shape-v1`,
`idle-inhibit`, `pointer-*`, `viewporter`.

It does **not** include:

| Protocol the spec relies on | In Mutter 51? | Consequence for us |
|---|---|---|
| `wlr-layer-shell` | ❌ (mutter#973, open since 2019) | Launcher must be a plain `xdg_toplevel` |
| `ext-foreign-toplevel-list-v1` | ❌ | No protocol window list |
| `wlr-foreign-toplevel-management` | ❌ | No protocol activate/close |
| `wlr-data-control` / `ext-data-control` | ❌ | No protocol clipboard monitoring |
| `ext-workspace` | ❌ | No protocol workspace switching |

Note also that `ext-foreign-toplevel-list-v1` is **list-only** even where it exists — its only
requests are `stop()` and `destroy()`. The gist's "issues `activate` and `close` requests directly to
the compositor" is wrong on every compositor. Activation is `xdg-activation-v1`; close has no
portable equivalent.

### 3.2 `org.gnome.Shell.Introspect` is not available to us

The obvious escape hatch — GNOME's own `GetWindows` DBus API — is gated. `js/misc/introspect.js`
defines `APP_ALLOWLIST = ['org.freedesktop.impl.portal.desktop.gtk',
'org.freedesktop.impl.portal.desktop.gnome']` and `GetWindowsAsync` runs every caller through
`this._senderChecker.checkInvocation(invocation)`. Third-party apps get "GetWindows is not allowed".

### 3.3 Therefore: the Shell extension is load-bearing

Compass already depends on a GNOME Shell helper extension exposing
`org.gnome.Shell.Extensions.Windows` (`services/window-manager/gnome/`) and
`org.gnome.Shell.Extensions.Clipboard` (`services/clipboard/gnome/`). On GNOME 50/51 **that is the
only mechanism available** for window switching, clipboard history and synthetic paste. It is
ported to Rust as `compass-shell`, not deleted.

### 3.4 What *does* work natively on GNOME

- **Global hotkeys — the one spec item that lands.** `xdg-desktop-portal-gnome` added the
  GlobalShortcuts backend in **48.rc**, improved it in **49.beta**, fixed activation-token delivery
  in **50.alpha**, and lists "various fixes to the Global Shortcuts portal" in **51.rc**. So `ashpd`
  GlobalShortcuts is the correct and only hotkey path on our target. `xx-hotkey-v1` and the X11
  backend are for *other* compositors and move to Phase 5.
- **`xdg-activation-v1`** for raising/focusing our own window and the app we launch.
- **`keyboard-shortcuts-inhibit-v1`** for grabbing keys while the launcher has focus — compass
  already has a `shortcut-inhibit` service; keep it.
- **Portals** generally: OpenURI, FileChooser, Screenshot, Secret are well covered by
  `xdg-desktop-portal-gnome`.

### 3.5 Restated goal for the GNOME integration

Not "eliminate the extension" — that is not on offer. Instead:

1. **Nothing in the critical path may require it.** App search, launch, calculator, emoji, snippets,
   file search and extensions must all work with **zero extension installed**. Only window
   switching, clipboard history and paste degrade.
2. **Shrink the extension to a minimal, versioned DBus contract** — ideally three methods and one
   signal — so a GNOME release breaks a 200-line extension, not the launcher. GNOME 51 ships
   16 September 2026; treat each GNOME release as a scheduled extension-compat task.
3. **Version the contract explicitly** (`org.gnome.Shell.Extensions.Vicinae` with a `Version`
   property) and make `compass-shell` degrade gracefully on mismatch instead of hard-failing.
4. **`vicinae doctor` must diagnose this precisely** — extension present / absent / version-mismatch,
   and what specifically is degraded as a result.

### 3.6 Bluefin's constraints

Bluefin is an immutable, OCI-composed Fedora Atomic desktop with **no traditional package manager**;
apps arrive as Flatpaks (GUI) or Homebrew (CLI). That has three consequences the spec does not
anticipate:

- **Flatpak is not a Phase-6 packaging chore, it is the Phase-0 development target.** If the dev
  loop is `cargo run` on a mutable host for six months, we will build something that does not work
  where it ships. The Flatpak manifest lands in Phase 0 and CI builds it from Phase 1.
- **`/dev/uinput` is unavailable**, so the spec's `wtype`/`dotool` paste fallback is dead on this
  target. Paste goes through the Shell extension, full stop.
- **Installing the Shell extension from inside a Flatpak is a real UX problem.** A sandboxed app
  cannot drop files into `~/.local/share/gnome-shell/extensions/` by default. Options: ship it on
  extensions.gnome.org and deep-link the user there; request
  `--filesystem=~/.local/share/gnome-shell/extensions`; or land it in the Bluefin image itself.
  **This needs a decision in Phase 0** (§10.7) — it gates the whole window/clipboard feature set on
  our first platform.
- Host app execution goes through `flatpak-spawn --host` / `OpenURI`; desktop-entry and icon
  indexing needs `--filesystem=host-os:ro` plus the `~/.local/share/{applications,icons}` reads.

---

## 4. Repository layout

```
compass/
├── CMakeLists.txt            # unchanged; C++ keeps building throughout
├── Cargo.toml                # [workspace] members = ["crates/*"]
├── rust-toolchain.toml
├── crates/
│   ├── compass-core/  compass-search/  compass-shell/   compass-wayland/
│   ├── compass-portals/ compass-ipc/   compass-xdg/     compass-platform/
│   ├── compass-ui/    compass-worker-host/
│   ├── compass-testkit/      # fixtures, mock GNOME Shell bus, parity harness
│   └── vicinae/              # the binary
├── packaging/flatpak/        # manifest + Bluefin CI, from Phase 0
├── src/                      # C++ tree, deleted directory-by-directory as parity lands
└── docs/rust-engine/         # this plan, REFERENCES.md, ADRs, PARITY.md
```

Build integration: `make dev-rust`, `make test-rust`, `make flatpak-rust`; an optional
`COMPASS_BUILD_RUST=ON` CMake switch so C++-only builds need no cargo until Phase 7. `make format`
gains `cargo fmt`; the lint gate is `cargo clippy -- -D warnings`. Nix gets a `crane`-based
derivation alongside the existing one.

---

## 5. Migration strategy

Two mechanisms stop this becoming an 18-month branch that never ships.

**(a) The engine switch.** From Phase 1 both engines are installed and selectable:

```
vicinae --engine=cpp    # default until Phase 7
vicinae --engine=rust
COMPASS_ENGINE=rust     # env override for CI and dogfooding
```

Same socket path, same config, same SQLite file. Users and CI can flip back mid-migration.

**(b) The parity ledger.** `docs/rust-engine/PARITY.md` — a checked-in table of every service,
builtin and CLI command with columns `C++ ✓ | Rust ✓ | parity test ✓ | C++ deleted ✓`. Nothing
leaves `src/` until its row is fully green. It is the definition of done for the project.

**Direction of travel:** vertical slice first (a launcher a person can actually use on Bluefin),
then breadth. rustcast already hands us the vertical slice's UI, so this is the cheap direction.

---

## 6. Phases

Each phase has a blocking, checkable exit gate.

### Phase 0 — Scaffolding and the Bluefin loop (≈1.5 weeks)

- Workspace, toolchain pin, CI (`build`, `fmt`, `clippy`, `test`) on Fedora 44/45 containers.
- **Flatpak manifest + `make flatpak-rust`, working from day one** (§3.6).
- Import rustcast under GPL-3.0 with provenance headers; strip every `objc2*` dep and all of
  `src/platform/macos/`; get `compass-ui` compiling as a library rendering a static list.
- `crates/compass-testkit` skeleton; empty `PARITY.md`.
- ADR-0001 Iced over Slint · ADR-0002 postcard over Cap'n Proto · ADR-0003 fluent-rs i18n ·
  **ADR-0004 GNOME Shell extension posture and distribution** (§3.5, §10.7) ·
  ADR-0005 Rhai as a third extension tier (§2.2, §10.9).

**Gate:** `cargo test --workspace` and `clippy -D warnings` green in CI; a Flatpak bundle builds and
launches a blank window on a Bluefin VM.

### Phase 1 — Thin vertical slice on GNOME (≈3 weeks)

Goal: `vicinae --engine=rust`, installed as a Flatpak on Bluefin, binds Super+Space via the portal,
opens a window, fuzzy-matches installed apps, launches one, closes. Nothing else.

- `compass-xdg`: desktop-entry parsing (port `lib/xdgpp` semantics), icon theme lookup — including
  host apps visible via `--filesystem=host-os:ro` and Flatpak exports.
- `compass-search`: `nucleo` behind a `FuzzySearchable`-equivalent trait.
- `compass-wayland`: plain `xdg_toplevel` via stock Iced/winit, centred, `xdg-activation-v1` to
  raise, `keyboard-shortcuts-inhibit-v1` while focused, dismiss on focus loss.
- `compass-portals`: `ashpd` GlobalShortcuts bind, including the first-run permission dialog.
- `compass-platform`: launch via `flatpak-spawn --host` with an `OpenURI` fallback.
- `compass-ui`: rustcast's shell wired to real results.

**Gate:** Suite 0 parity (§8.1) green for app-search ranking on the 500-entry corpus; runs from a
Flatpak on Bluefin with GNOME 50 **and** 51; idle RSS < 30 MB; **works with no Shell extension
installed** (§3.5.1).

### Phase 2 — IPC, CLI, doctor (≈2 weeks)

- `compass-ipc` socket + framing; `vicinae toggle`, `vicinae ext list --json`.
- `vicinae doctor`: portal backends, DBus, socket, Flatpak permissions, **and Shell-extension
  presence/version** (§3.5.4), with `--check-only` exit codes.
- Single-instance handling and `$XDG_RUNTIME_DIR` socket lifecycle inside the sandbox.

**Gate:** IPC round-trip p99 < 0.5 ms (criterion); `doctor` output diffed against the C++ build on
the same machine; `doctor` correctly reports each degradation with the extension uninstalled.

### Phase 3 — GNOME Shell integration (≈3 weeks)

- `compass-shell`: `zbus` client for `org.gnome.Shell.Extensions.{Windows,Clipboard}`.
- Shrink and version the extension's DBus contract per §3.5.2–3; publish the versioned interface XML
  in-tree so the extension and the engine can be reviewed against one another.
- Window switcher; clipboard history with the encrypted SQLite store (port `lib/crypto` semantics);
  paste via the extension.
- Graceful degradation paths and user-visible messaging for every missing capability.

**Gate:** mock-Shell-bus suite (§8.4a) green; clipboard DB readable and writable by both engines
interchangeably; extension-absent and version-mismatch paths both tested; a week of dogfooding by
≥2 people on Bluefin.

### Phase 4 — Extension host (≈6–8 weeks, the hard one)

- **Carve out `compass-extension-api` first**, before the Node host is written against it: the
  capability registry, the view tree and action dispatch, with no knowledge of Node, JSON-RPC or
  Rhai. This is the seam that makes the Rhai tier (§2.2) a binding exercise instead of a parallel
  stack. It costs perhaps three days now and saves weeks in Phase 5.
- `compass-worker-host` spawns `vicinae-worker-ts` per extension over UDS with JSON-RPC 2.0,
  consuming `compass-extension-api` rather than defining its own view model.
- **`src/typescript/` is not rewritten.** The React reconciler and `@raycast/api` shim keep working;
  only the host changes. Any change forced on the SDK is a design smell — escalate it.
- Sandbox: **Landlock** for the filesystem boundary (unprivileged, no bind mounts — a better fit for
  a Flatpak than the spec's read-only mounts), `seccompiler` for the syscall filter, cgroups v2 for
  the 256 MB cap. Note that we are sandboxing *inside* an already-sandboxed Flatpak; verify the
  nesting works on Bluefin early rather than late.
- OAuth, local-storage, toast and navigation host APIs.

**Gate:** Suite 1 (§8.2) — top 25 Raycast store extensions plus every Vicinae store extension run
unmodified **inside the Flatpak**; every negative sandbox test fails closed; and
`compass-extension-api` compiles and passes its tests with `compass-worker-host` removed from the
dependency graph — the cheap mechanical proof that the seam is real.

### Phase 5 — Breadth, and the second compositor (≈8–10 weeks, parallelisable)

Three tracks that do not block each other:

*Track A — builtins:* calculator · clipboard · emoji/glyph · file search (+ `file-indexer`) · font ·
media control · power management · shortcuts · snippets · system · theme · script commands ·
dmenu · store front-ends · window/workspace · developer tools.

**Browser tab search and switching is not in this list.** It is out of scope for the port and
becomes an extension — see [ADR-0008](./adr/0008-browser-control-is-an-extension.md). It is a
browser feature surfaced in a launcher, with no coupling to the compositor, clipboard or index, and
its churn (manifest v2 → v3, per-browser layouts, store review) is not churn we control. Note the
consequence: if no extension exists by Phase 7, this is a **feature regression at cutover** and
belongs in the release notes next to the macOS/Windows narrowing.

*Track B — compositor #2 (wlroots — Hyprland/Sway/niri):* add `wlr-layer-shell` via
[`iced_layershell`](https://crates.io/crates/iced_layershell), `ext-foreign-toplevel-list-v1` +
`wlr-foreign-toplevel-management`, `wlr-data-control` clipboard, and the `xx-hotkey-v1` backend from
#1936 (necessary because `xdg-desktop-portal-wlr` ships **no** GlobalShortcuts backend). KDE is a
third target after that.

*Track C — Rhai extension tier (§2.2).* Independent of both, once `compass-extension-api` exists:
`compass-script` with a hardened engine (`Engine::new_raw()`, explicit package, no
`FileModuleResolver`, the full set of `set_max_*` limits, `on_progress` budget termination); the
capability-gated function registry; `spawn_blocking` execution with a wall-clock timeout; script
discovery and hot reload; and first-party example scripts with authoring docs. The tier ships only
when the examples are good enough that someone can copy one and be productive — an empty tier is
worse than no tier.

**Gate:** parity ledger ≥ 95% green, with every ported group's Catch2 tests ported to Rust (§8.3).
For Track C: the Rhai sandbox negative tests (§8.2) all fail closed, and at least four first-party
example scripts ship with docs.

Note that a row going green does **not** mean its C++ directory is deleted. An earlier draft of this
plan said it did, which was simply wrong: both engines ship side by side until Phase 7, so the C++
engine still needs its own matcher, IPC and parsers however complete the Rust ones are. Deletion is
Phase 8 work, and `PARITY.md` marks such rows `⏳`.

### Phase 6 — Packaging breadth (≈2 weeks)

Flatpak already exists from Phase 0; this phase adds back AppImage, Arch, Nix and the
`vicinae.json` declarative config with a published JSON Schema plus migration from today's config.

**Gate:** Suite 5 (§8.6) green across all outputs.

### Phase 7 — Cutover (≈2 weeks)

Default flips to `--engine=rust` on Linux; the C++ engine stays one release behind a flag.
**macOS and Windows are explicitly out of scope for cutover** — the C++ engine remains the shipping
build there until a follow-up project. Compass supports all three today, and a Linux-only Rust
default is a visible narrowing: say so loudly in the release notes.

**Gate:** one full release cycle with no P0 regressions.

### Phase 8 — Removal

Delete `src/server`, the C++ `src/lib`, and the Linux CMake targets. Keep macOS/Windows targets
until their own migration. Repo becomes Rust-primary.

---

## 7. Rough schedule

| Phase | Effort (1 FTE) | Parallelises to |
|---|---|---|
| 0 Scaffolding + Flatpak loop | 1.5 wks | 1 |
| 1 GNOME vertical slice | 3 wks | 2 |
| 2 IPC / CLI / doctor | 2 wks | 2 |
| 3 GNOME Shell integration | 3 wks | 2 |
| 4 Extension host | 6–8 wks | 2 |
| 5 Breadth + compositor #2 | 8–10 wks | 4+ |
| 6 Packaging breadth | 2 wks | 1 |
| 7 Cutover | 2 wks | — |
| **Total** | **~7 months serial** | **~4 months at 3–4 FTE** |

Order-of-magnitude only. Phase 4 is the one most likely to double.

---

## 8. Testing plan

The gist specifies five suites. They are good and are adopted below, but they share one blind spot:
**nothing in them compares the Rust implementation against the C++ implementation it replaces.** In
a strangler rewrite that differential is the highest-value test available, so it is added as
Suite 0 and it is the gate that matters most in Phases 1–5. The Wayland-mock suite is also
re-weighted: on a GNOME-first target, a **mock GNOME Shell DBus service** catches far more than a
mock wlroots compositor.

### 8.1 Suite 0 — Cross-implementation parity (new; the safety net)

A golden-corpus differential harness in `crates/compass-testkit`.

**Corpora, checked in under `tests/corpus/`:**
- ~500 real `.desktop` files: localised `Name[xx]`, all `Exec` field codes (`%f %F %u %U %i %c %k`),
  `TryExec`, absolute and themed icons, `NoDisplay`, `OnlyShowIn`, malformed and non-UTF-8 entries.
  **Harvest from a live Bluefin image** — host RPM apps, Flatpak exports and Homebrew entries
  together — since that is the exact mix our first users have.
- ~2,000 query/expected-ranking pairs harvested from `src/lib/fuzzy/tests` and
  `src/file-indexer/tests/query-quality.cpp`.
- A recorded clipboard/history fixture and a set of theme files.

**Runner:** for each item, run the operation against both engines (`vicinae --engine=cpp|rust --json`)
and diff structured output.

**Verdicts:** `identical` / `known-divergence` (must cite a ledger row and a rationale) /
`regression` (fails CI). Divergences are *declared*, never discovered.

Plus two compatibility checks that matter because both engines coexist for months:
- *Schema compat:* open the same SQLite file with both engines in both orders; assert no corruption
  and no lost rows.
- *Config compat:* every config in the corpus yields an equivalent effective configuration in both.

This is also how the ledger's "parity test ✓" column gets filled — a row cannot go green without a
Suite-0 case.

### 8.2 Suite 1 — Raycast extension API conformance

- `@vicinae/test-harness` in `src/typescript/`: drives `<List>`, `<Detail>`, `<Form>`,
  `<ActionPanel>`, `<Grid>`, `<MenuBarExtra>` and asserts the reconciler's state tree serialises to
  the expected RPC frames.
- Hooks: `useFetch`, `useCachedState`, `useLocalStorage`, `useExec`, `useNavigation`, `usePromise`,
  `useSQL`.
- Host built-ins mocked and asserted: `environment.assetsPath`, `environment.supportPath`,
  `environment.isDevelopment`, preference resolution, OAuth PKCE.
- **Persistence across restart:** write via `useLocalStorage`, kill the worker, restart, read back.
  Sandboxed state dirs usually break exactly here.
- **Real-extension corpus:** top 25 Raycast store extensions by installs plus every Vicinae store
  extension, installed and driven headlessly through one command each, snapshotting the first frame.
  **Run this inside the Flatpak**, not on a mutable host — extensions that shell out are precisely
  what the sandbox breaks.
- **Negative sandbox tests (Node):** an extension attempting `fork`, raw sockets, writes outside its
  state dir, or a 512 MB allocation must fail closed with a diagnostic, not crash the core.
- **Negative sandbox tests (Rhai), from Phase 5:** a script that did not declare a capability cannot
  reach it — the function is simply absent from its scope. Plus `import` resolves nothing (the
  default `FileModuleResolver` must not be installed); an infinite loop is terminated by the
  `on_progress` operation budget; a script exceeding the wall-clock timeout is killed without
  stalling the render thread; and `set_max_string_size` / `set_max_array_size` /
  `set_max_call_levels` / `set_max_expr_depths` each reject their overflow case.
- **Shared-seam test:** the same fixture extension expressed once in TypeScript and once in Rhai
  must produce the same view tree through `compass-extension-api`. This is the regression test that
  keeps the two tiers from drifting.

`npm --prefix src/typescript test -- --filter=raycast-api-conformance`

### 8.3 Suite 2 — Upstream test harvest ("steal and port")

Compass's Catch2 suites encode years of bug fixes. They are an asset, not legacy.

| Existing suite | Port target | Method |
|---|---|---|
| `src/lib/fuzzy/tests` | `compass-search` | direct port + `proptest` ranking invariants |
| `src/lib/xdgpp/tests` (entry, mime, locale, file-uri, bookmark, special, xdg-terminal-exec, …) | `compass-xdg` | direct port — **the highest-value harvest in the repo** |
| `src/lib/crypto/tests` | `compass-core` | direct port; verify ciphertext compat with existing DBs |
| `src/lib/script-command/tests` | `compass-core` | direct port |
| `src/lib/glyph/tests` | `compass-core` | direct port |
| `src/lib/vicinae-ipc/tests` | `compass-ipc` | re-express against the new framing |
| `src/file-indexer/tests/query-quality.cpp` | `compass-platform` | becomes a Suite-0 corpus |
| `src/snippet/tests` | `compass-core` | direct port |

```rust
proptest! {
    #[test]
    fn desktop_entry_parsing_never_panics(s in "\\PC*") {
        let _ = compass_xdg::DesktopEntry::parse_str(&s);
    }
}
```

**Rule:** a C++ test file may only be deleted in the same PR that adds its Rust equivalent, and the
PR body must show both passing.

### 8.4 Suite 3 — Desktop integration

Re-weighted for a GNOME-first target, in three tiers.

**(a) Mock GNOME Shell bus — per-PR, the workhorse.** A fake `org.gnome.Shell.Extensions.{Windows,
Clipboard}` service on a private DBus in `compass-testkit`. Cheap, fast, no display server. Assert:
window list mirrors state across add/remove/rename races; clipboard signals produce correct history
rows; **extension absent** and **version mismatch** both degrade correctly and surface the right
`doctor` diagnosis; DBus disconnect mid-session reconnects.

**(b) Headless GNOME session — nightly.** `gnome-shell --headless --virtual-monitor` in a Fedora
44/45 container running a scripted 10-step session against both GNOME 50 and 51. This is the tier
that catches real portal behaviour, the GlobalShortcuts permission dialog, and
`xdg-activation-v1` focus semantics.

**(c) Wayland mock compositor — from Phase 5.** The spec's `smithay` headless fixture in
`compass-testkit/src/wayland_mock.rs`, for the wlroots track: layer-shell anchors and margins across
single/dual/mixed-DPI outputs, `ext-foreign-toplevel-list-v1` events, focus-loss dismissal, and
correct degradation when a protocol is absent. Deferred until there is wlroots code to test.

### 8.5 Suite 4 — Benchmarks and resource regression

`criterion` benches with **SLAs enforced as CI failures**, not advisory numbers:

| Metric | SLA | Source |
|---|---|---|
| Fuzzy search, top-20 of 10,000 items | < 2.0 ms | gist spec |
| IPC round-trip, local UDS | < 0.5 ms | gist spec |
| Cold start to first frame | < 120 ms | new — the number users feel |
| Idle RSS | < 30 MB | gist spec |
| Peak RSS, 10k index + 3 extensions | < 150 MB | new |

The spec claims sub-30 MB but proposes no test for it; without a gate the claim decays. Track RSS
per commit, fail on >5% regression. **Measure inside the Flatpak** — sandbox overhead is real and
the number users see is the sandboxed one.

Plus `insta` snapshot tests rendering views to a headless framebuffer. Keep these *few* and
semantic (results list, empty state, detail view, form). Large pixel-snapshot suites get
rubber-stamped and stop catching anything.

### 8.6 Suite 5 — Packaging and deployment

- **Bluefin end-to-end, the headline test:** install the Flatpak on a Bluefin image in CI, bind the
  hotkey through the portal, launch a host RPM app, a Flatpak app and a Homebrew binary, and assert
  no permission panic. This is our first target — it belongs in CI from Phase 1, not Phase 6.
- Flatpak conformance via `flatpak-builder`; host execution through `flatpak-spawn --host` /
  `OpenURI`.
- `vicinae doctor --check-only` asserted to detect present **and absent** portals, protocols, DBus,
  sockets and the Shell extension. Test both directions — a doctor that always says "fine" is worse
  than no doctor.
- Install-matrix smoke for AppImage, Arch, Nix (from Phase 6): `--version` + `doctor`.

### 8.7 Pre-flight command

```sh
cargo test --all-targets --workspace
cargo clippy --all-targets --workspace -- -D warnings
cargo bench --bench slas -- --save-baseline pr
npm --prefix src/typescript test
cargo run -p compass-testkit --bin parity -- --corpus tests/corpus --engines cpp,rust
flatpak run com.vicinae.Vicinae -- doctor --check-only
```

### 8.8 CI wiring

Four tiers, ordered by how real the environment is and how much it costs. A tier only exists if the
tier below it cannot catch the bug.

**Tier 1 — per PR, no display server.** Pure logic; seconds to minutes.

| Job | Trigger | Budget |
|---|---|---|
| build + clippy + fmt + unit | every PR | < 8 min |
| Suite 0 parity (fast corpus) | every PR touching `crates/` | < 5 min |
| Suite 3a mock GNOME Shell bus | every PR touching `compass-shell` | < 3 min |
| Suite 1 TS conformance | every PR touching `src/typescript` or `compass-worker-host` | < 10 min |
| Suite 4 benches | every PR informational; **blocking on `main`** | < 10 min |

**Tier 2 — per PR, headless GNOME in a container.** `gnome-shell --headless --virtual-monitor` in a
Fedora 44/45 container gives real Mutter and a real `xdg-desktop-portal-gnome` without a VM, so it
catches most integration bugs at container cost. Reach for this before reaching for QEMU.

| Job | Trigger | Budget |
|---|---|---|
| Flatpak build | every PR from Phase 1 | < 12 min |
| Headless GNOME session smoke | every PR from Phase 1 | < 10 min |

**Tier 3 — merge queue, a real Bluefin VM.** See §8.9. The only tier that tests what users install.

**Tier 4 — nightly, unbounded.** Suite 0 full corpus; the GNOME 50 **and** 51 matrix; Suite 5
packaging; the wlroots compositor matrix from Phase 5; perf and RSS trends.

`sccache` + `cargo-nextest` keep the per-PR budget honest.

### 8.9 The merge-queue tier: a real Bluefin VM under QEMU

Tiers 1 and 2 never boot the operating system we ship on. A launcher is a system-integration
product — portals, session, compositor, Flatpak sandbox, Shell extension — so the things most
likely to break are exactly the things a container cannot exercise. The merge queue is the right
home for that: it runs after review, on the merged result, so a slow boot costs throughput rather
than iteration speed.

**Substrate.** Bluefin ships as a bootc OCI image (`ghcr.io/ublue-os/bluefin:stable`).
[`bootc-image-builder`](https://github.com/osbuild/bootc-image-builder) converts it to a `qcow2` —
the same two-stage path Bluefin uses to produce its own installable media. The VM under test is
therefore not an approximation of the target; it is the target, built the way the target is built.

**Trigger.** GitHub Actions' `merge_group` event. Tiers 1 and 2 stay on `pull_request`.

**Shape of a run:**

1. Build the Flatpak (reuse the Tier-2 artifact).
2. `bootc-image-builder` → `qcow2`, cached per Bluefin image digest so most runs skip the build.
3. Boot under QEMU with a virtual display; autologin into a GNOME session.
4. Provision over SSH: install the Flatpak, install the Shell extension, restart the session.
5. Drive a scripted session and assert:
   - `vicinae doctor --check-only` exits 0 and reports the capabilities we expect;
   - the GlobalShortcuts portal binds, including the first-run permission dialog;
   - the launcher opens, filters, and launches a host RPM app, a Flatpak app and a Homebrew binary;
   - window switching and clipboard history work **with** the Shell extension;
   - **and everything still works with the extension uninstalled**, degrading exactly as `doctor`
     claims. That is the §3.5.1 promise, and a VM is the only place it can be checked.
6. Capture screenshots and the journal as artifacts on failure.

**Assert over IPC, not over pixels.** Drive assertions through our own IPC socket and `doctor`, and
keep a handful of screenshots as human-readable artifacts only. Pixel-scraping a desktop session is
the classic way to build an e2e suite everyone learns to ignore.

**The KVM problem, which is the real constraint.** GitHub-hosted runners expose no `/dev/kvm` and
[do not support nested virtualisation](https://github.com/orgs/community/discussions/8305), so QEMU
there falls back to TCG emulation — roughly an order of magnitude slower, which turns a desktop boot
into many minutes. Options, in preference order:

1. **Depot CI sandboxes**, where `/dev/kvm` is enabled by default. We already use Depot for the C++
   builds, so this is the smallest change — but their *standard* GitHub Actions runners are not the
   same product as their CI sandboxes, so confirm this before designing around it.
2. A **self-hosted runner** on bare metal or a nested-virt-capable cloud instance.
3. **Unaccelerated TCG**, accepted as slow. Viable precisely because this tier is out of the PR
   loop, and a reasonable way to start before committing to infrastructure.

**Promote it; do not start with it.** A flaky VM job in the merge queue blocks merges for everyone,
and desktop-session e2e is the most flake-prone thing we will build. Run it nightly first and move
it into the merge queue only once it has been stable for a couple of weeks. Then hold it to the same
rule as everything else: a failure is a bug until proven otherwise, and "flake" is not a root cause.

Fedora solves this problem at scale with [openQA](https://openqa.fedoraproject.org/), worth knowing
about if our own harness starts to sprawl — but it is a much heavier commitment and not where we
should start.

---

## 9. Risks

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| GNOME 51 (16 Sep 2026) breaks the Shell extension | **Certain, recurring** | High | §3.5 — tiny versioned extension, graceful degradation, a scheduled compat task each GNOME cycle, and nothing critical-path behind it |
| Can't install the Shell extension from a Flatpak | High | High | ADR-0004 in Phase 0; extensions.gnome.org deep-link, a `--filesystem` hole, or shipping it in the Bluefin image |
| Phase 4 (extension host) overruns | High | High | Freeze `src/typescript`; if the Rust host stalls, ship Phases 1–3 with the **C++ extension host bridged over IPC** as a transitional hybrid |
| Sandbox-in-sandbox (Landlock/seccomp inside Flatpak) doesn't work on Bluefin | Medium | High | Prove it in a Phase-0 spike, before Phase 4 depends on it |
| Iced can't match Qt/QML polish (fonts, **IME**, a11y, RTL) | Medium | High | Prove in Phase 1 with the real theme set. Test IME and screen-reader support early, not late — GNOME users notice. ADR-0001 revisitable until Phase 5 |
| Losing translations | Medium | Medium | §2.1 — `.ts` → `.ftl` converter in Phase 0, catalogue coverage checked in CI |
| macOS/Windows users stranded | Certain | Medium | Stated in Phase 7; the C++ engine remains their build. Do not let it be a surprise |
| Divergence from upstream vicinaehq/vicinae becomes unmergeable | High | Medium | Accept it: after Phase 1 this is a hard fork in practice. Decide deliberately in Phase 1, not by drift |
| A 100k-LOC rewrite never finishes | Medium | Fatal | The parity ledger + engine switch mean partial completion is still shippable value |
| Sandbox breaks legitimate extensions | Medium | Medium | Ship log-only filters first, enforce a release later |
| A third extension API surface (Rhai) drifts from the TS one | Medium | Medium | `compass-extension-api` as a single capability layer, carved out in Phase 4 and proven by the dependency-graph gate. If that seam does not materialise, **drop the Rhai tier** rather than maintain two stacks |
| Rhai tier ships into an empty ecosystem and nobody uses it | Medium | Low | Cheap if the seam exists; gate the tier on four good first-party examples and treat §10.9 as a real go/no-go |
| A Rhai script hangs the UI | Medium | Medium | `spawn_blocking` only, operation budget via `on_progress`, wall-clock timeout, bounded blocking-thread pool |

---

## 10. Decisions and remaining questions

The questions that were open when this plan was written have been decided and recorded as ADRs in
[`adr/`](./adr/). Summary:

| Was | Decided | ADR |
|---|---|---|
| Iced or Slint? | Iced 0.14 | [0001](./adr/0001-iced-over-slint.md) |
| Cap'n Proto or something simpler? | postcard now, Cap'n Proto held in reserve behind the 0.5 ms SLA | [0002](./adr/0002-postcard-over-capnproto.md) |
| i18n, absent from the spec | fluent-rs, with the Qt Linguist catalogue converted rather than lost | [0003](./adr/0003-fluent-for-i18n.md) |
| **How does a Flatpak install the Shell extension?** | extensions.gnome.org **and** baked into the Bluefin image; explicitly **not** a `--filesystem` hole into GNOME's directory | [0004](./adr/0004-gnome-shell-extension-distribution.md) |
| Is the Rhai tier worth it? | Build the seam now; the tier is a product go/no-go at the end of Phase 4 | [0005](./adr/0005-rhai-seam-now-tier-later.md) |
| The fuzzy coherence gap | Reconstruct the signal over nucleo's indices, rather than raising the gate or accepting looser matching | [0006](./adr/0006-fuzzy-coherence-classifier.md) |
| Fork posture, branding, platform scope, GNOME versions | Hard fork acknowledged; `vicinae` user-facing names kept; Linux-first with macOS/Windows on the C++ engine; GNOME 50 **and** 51 in CI | [0007](./adr/0007-fork-posture-and-platform-scope.md) |
| Does browser control belong in the core? | No — it becomes an extension and leaves the port's scope entirely | [0008](./adr/0008-browser-control-is-an-extension.md) |

### Still genuinely open

1. **Team size.** The schedule in §7 swings between four and seven months on this work alone. Nobody
   can answer this from inside the plan.
2. **rustcast relationship** — one-time seed (what the plan assumes and what the crate split
   reflects), or an ongoing sync? The latter would constrain the crate boundaries in §2 and cost
   design freedom. Assumed one-time until someone says otherwise.
3. **Whether to report the six C++ desktop-entry bugs upstream.** ADR-0007 says we should as a
   courtesy; someone has to actually do it. See PARITY.md for the list, one of which is an unbounded
   loop reachable from any malformed `.desktop` file on disk.

### Decided by doing, not by discussion

Some things in the original list resolved themselves once code existed, and are recorded here so
they are not re-litigated: `nucleo` over hand-rolling a matcher; a caller-owned buffer in
`FuzzySearchable` rather than returning `Vec` or a GAT-flavoured iterator; total-order ranking so
results are deterministic; and raw bytes rather than `String` in the corpus API, because part of the
corpus is deliberately not valid UTF-8.

## 11. Current state

Updated as work lands. See [`PARITY.md`](./PARITY.md) for the per-subsystem ledger and
[`adr/`](./adr/) for the decisions.

### Done

- **Workspace and CI.** Six crates, pinned 1.94.1, edition 2024, `unsafe_code` forbidden and
  `clippy::all` denied workspace-wide. Rust CI workflow, Makefile targets kept separate from the C++
  ones. Verified in a clean worktree checkout, not just in the dirty tree.
- **Corpora.** 8 real `.desktop` entries plus 19 synthetic edge cases, and a harvester script for
  growing the real half on a machine that has applications installed. Corpus files are `-text` in
  `.gitattributes`, with a test that fails loudly if a checkout ever normalises the CRLF and
  Latin-1 fixtures into fixtures that test nothing.
- **`compass-xdg`** — desktop-entry, locale, value, reader and exec layers, with all 47 in-scope
  C++ cases ported verbatim.
- **`compass-search`** — fuzzy matching on `nucleo`, with the C++ ordering suite ported.
- **ADRs 0001–0007**, including the two that were blocking: how a Flatpak installs the Shell
  extension, and how to close the fuzzy coherence gap.
- **Flatpak manifest** for the Bluefin target — syntax-validated only; never built.
- **i18n converter** — 7,347 messages across 7 locales, all parsing with the real `fluent-syntax`
  crate. ADR-0003's claim that the donated translations survive is now demonstrated rather than
  asserted.

### What the dev container cannot verify

Worth stating plainly, because "the tests pass" means less than it sounds like until these are
covered. The container has no display server, no `flatpak`, no `qemu`, and no `/dev/kvm`:

- the Wayland surface and anything in `compass-ui`;
- the GlobalShortcuts portal path — an `ashpd` call needs a portal implementation on the bus;
- the Flatpak build, and therefore every claim in `packaging/flatpak/`;
- the Tier-3 VM tier in §8.9.

It *can* run a real DBus session bus (`dbus-run-session` works), so the GNOME Shell integration and
its mock-bus suite are genuinely testable here. That is why Phase 3's testing is further along than
Phase 1's, which inverts the plan's order — deliberately, because verified work beats sequenced
work.

### Blocked on someone with access

- **GitHub Actions is not enabled on this fork.** No checks run on any PR. Every claim above was
  verified locally, which does not scale past one person. This is the single highest-value
  unblocking action available.
- **Run the corpus harvester on a real Bluefin box.** The synthetic corpus is a model of the spec,
  not of reality.
- **Phase 0 spikes A and B** (portal hotkey on real GNOME; Landlock + seccomp inside a real
  Flatpak) both need an environment this container cannot provide, and Phase 4 should not be
  designed further until B is answered.

## 12. Immediate next steps


1. Answer §10.7 (extension distribution) and §10.1–2 (fork posture, branding) — they change file
   names and architecture, so they are cheap now and expensive later.
2. Land Phase 0 as one PR: workspace, CI, **Flatpak manifest**, ADRs 0001–0004, empty parity ledger.
3. Build the Suite-0 corpus **before** writing Phase 1 code, harvesting `.desktop` files from a real
   Bluefin image and ranking pairs from the existing Catch2 suites. A day of work that makes every
   later phase verifiable.
4. Run two Phase-0 spikes in parallel, each timeboxed to a week:
   - **Spike A:** Iced app in a Flatpak on Bluefin, binding Super+Space through the GlobalShortcuts
     portal and raising itself with `xdg-activation-v1`. Answers "does the GNOME path work at all".
   - **Spike B:** Landlock + seccomp around a Node child process *inside* a Flatpak. Answers whether
     Phase 4's sandbox design is viable before we build on it.
