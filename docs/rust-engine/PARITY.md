# Parity ledger

The definition of done for the Rust engine transformation (#2). Every row must be fully green
before its C++ source is deleted, and **nothing leaves `src/` until it is**.

Columns:

- **C++ ✓** — the behaviour exists in the C++ engine today. Effectively always true; it is here so a
  row with a false in this column stands out as new functionality rather than a port.
- **Rust ✓** — implemented in the Rust engine.
- **parity test ✓** — covered by a Suite 0 differential case (#12) or, where a differential is not
  meaningful, by ported tests from the C++ suite (`PLAN.md` §8.3). A checked box here means a test
  would *fail* if the two engines diverged.
- **C++ deleted ✓** — the C++ source is gone. `⏳` means "green, but the C++ engine still needs this
  code because it is still the shipping engine". **Deletion cannot happen before the Phase 7
  cutover, no matter how green the row is** — see the note below.

A row may go green with a **declared divergence** instead of exact parity: record it in the
Divergences section below with a rationale. Divergences are declared, never discovered.

## Scope of this ledger

**Every row below is the Linux port.** That is Phases 0–8, and it is what the row set was built to
track. It is not the whole port, and reading it as one would badly understate what remains: a fully
green ledger here is the state at the end of Phase 8, with Qt still in the repository.

The reason is that the non-Linux surface is not shaped like these rows. Rows here map a C++
directory to a Rust crate, because Linux behaviour lives in Linux files. macOS and Windows
behaviour mostly does not — it lives in conditional compilation inside files that compile
everywhere, so there is no directory to put in the left-hand column:

| | dedicated `.cpp` | `#ifdef` sites | shared files touched |
|---|---:|---:|---:|
| Linux | 59 | 72 (`Q_OS_LINUX`) | 30 |
| Windows | 33 | 100 (`Q_OS_WIN`) | 43 |
| macOS | 3 | 102 (`Q_OS_MAC`) | 38 |

Counted by attributing each translation unit to the `if (APPLE)` / `if (WIN32)` /
`if (UNIX AND NOT APPLE)` block that lists it in `src/server/CMakeLists.txt`, and by grepping the
guards. 61 shared files carry at least one conditional; `server.cpp` alone has 29.

So the macOS and Windows work is tracked as **conditional sites resolved**, not as directories
ported, and it lives in [PLAN.md](./PLAN.md) §6 Phases 9 and 10 rather than here. A row in this
ledger going green says nothing about either.

Update this file in the same PR that changes a box. It is meant to be read in standup.

### A correction to the plan's deletion rule

`PLAN.md` §6 Phase 5 said each ported group's `src/` directory is deleted "in the same PR that turns
the row green". That cannot be right, and following it would break the build: both engines ship side
by side until Phase 7, so the C++ engine still needs its own matcher, IPC and parsers no matter how
complete the Rust replacements are.

The rule is therefore: **a row goes green in the PR that ports it and proves parity; the `C++
deleted` column is worked at Phase 8, after cutover.** `⏳` marks rows that are done but waiting on
that. The plan has been corrected.

### Progress at a glance

| Crate | Tests | State |
|---|---|---|
| `compass-core` | 753 | app index, frecency, config, root search, glyphs, snippets, toasts, quicklinks, the extension boilerplate generator, the image fetch queue, the confirm dialog, volume and mute, the paste handoff, the telemetry record, update checks, the news notices, the selected text, the file dialog, both extension stores, emoji metadata, the snippet input server's framing, the indexer's entry filter and query policy |
| `vicinae` | 184 | CLI, an 11-check `doctor`, and **the engine daemon** |
| `compass-worker-host` | 187 | the extension host: framing, sandboxed spawn, 45 of tsapi's 49 methods, and the real runtime |
| `compass-xdg` | 150 | desktop entries, locale, exec, reader, mimeapps, bookmarks — scope gaps listed below |
| `compass-clipboard` | 74 | history store, ingest, migrations; stored enums pinned to the C++ header |
| `compass-extension-api` | 73 | view tree, derived identity, diff, dispatch, capabilities, controlled inputs |
| `compass-ipc` | 73 | framing, transport, single-instance |
| `compass-search` | 58 | fuzzy, plus an exact port of fzf's coherence rule |
| `compass-portals` | 54 | XDG portals; availability is a three-state outcome, not a boolean |
| `compass-shell` | 46 | GNOME Shell DBus client; tests spawn a real `dbus-daemon` |
| `compass-ui` | 29 | the launcher window and its views |
| `compass-crypto` | 24 | AES-GCM and HKDF; cross-decrypted against the C++ probe per-PR |
| `compass-sandbox` | 23 | Landlock, a seccomp denylist, and the launcher that applies them to itself |
| `compass-local-storage` | 20 | the extension key-value store, lossy typing and all |
| `compass-media` | 13 | MPRIS players, with the timeout the C++ has for a reason |
| `compass-power` | 11 | logind; one C++ bug deliberately not reproduced |
| `compass-db` | 10 | the shared migration runner and the `vicinae` schema |
| `compass-oauth-store` | 9 | the extension token store |
| `compass-sqlcipher-sys` | 8 | SQLCipher and the vendored tokenizer; connection pragmas pinned to the C++ |
| `compass-testkit` | 8 | corpora — **757 desktop entries, 738 harvested from real hosts** |
| `compass-notify` | 7 | desktop notifications over D-Bus |
| `compass-platform` | 6 | the launcher seam (ADR-0013) |
| `compass-platform-linux` | 26 | the launcher, and the uinput virtual keyboard's protocol |
| `compass-wayland` | 2 |  |
| **Total** | **1,855** | what `make check-rust` reports, doctests included, all green under fmt and clippy `-D warnings` |

The per-crate column is measured with `cargo test -p <crate> --all-targets` and sums to 1,848;
the 1,855 is the workspace figure `make check-rust` prints, which additionally covers doctests and
harnesses not attributable to a single package. Both numbers are given rather than one reconciled
figure, because quietly picking whichever is larger is how a count stops meaning anything.

## Progress

Scaffolding, corpora and CI are in place, and **twenty** crates have landed: **1,151 tests** across
the workspace, all green. `compass-db` (the shared migration runner and the `vicinae` schema, extracted from
`compass-clipboard`), `compass-local-storage`, `compass-oauth-store` and `compass-sandbox` are the four newest. An earlier revision of this
paragraph said nine crates and 597 tests, and both had drifted — `compass-clipboard`,
`compass-crypto`, `compass-sqlcipher-sys`, `compass-platform`, `compass-platform-linux`,
`compass-wayland` and `compass-worker-host` were missing from the table entirely, and the corpus
line still read 115 entries against an actual 757.

Almost no row is fully green, and no C++ directory may be deleted yet — see the partial markers and
the divergences below. 🟡 means implemented but not to the full scope of the C++ source. A green
`Rust ✓` with a 🟡 `parity test ✓` means the code exists and is tested as far as this container can
test it; `compass-portals` is the clearest case, since no amount of local testing can tell us
whether a real GNOME session grants the shortcut we ask for.

## Libraries and standalone binaries

| C++ source | Rust home | Phase | C++ ✓ | Rust ✓ | parity test ✓ | C++ deleted ✓ |
|---|---|---|:-:|:-:|:-:|:-:|
| `src/lib/xdgpp` | `compass-xdg` | Phase 1 | ✅ | 🟡 | ✅ | ❌ |
| `src/lib/fuzzy` | `compass-search` | Phase 1 | ✅ | ✅ | ✅ | ⏳ |
| `src/lib/crypto` | `compass-crypto` | Phase 3 | ✅ | ✅ | ✅ | ⏳ |
| `src/lib/glyph` | `compass-core` | Phase 5 | ✅ | 🟡 | ✅ | ❌ |
| `src/lib/script-command` | `compass-core` | Phase 5 | ✅ | ✅ | ✅ | ❌ |
| `src/lib/vicinae-ipc` | `compass-ipc` | Phase 2 | ✅ | ✅ | 🟡 | ⏳ |
| `src/lib/figura` | `compass-ipc` | Phase 2 | ✅ | n/a | n/a | ⏳ |
| `src/lib/common` | `compass-core` | Phase 2 | ✅ | 🟡 | ✅ | ❌ |
| `src/lib/linux-utils` | `compass-platform-linux` | Phase 2 | ✅ | 🟡 | ✅ | ❌ |
| `src/lib/soulver` | `—` | n/a (macOS) | ✅ | ❌ | ❌ | ❌ |
| `src/cli` | `crates/vicinae` | Phase 2 | ✅ | 🟡 | 🟡 | ❌ |
| `src/file-indexer` | `compass-platform` | Phase 5 | ✅ | ❌ | ❌ | ❌ |
| `src/data-control-server` | `compass-wayland` | Phase 5 | ✅ | ❌ | ❌ | ❌ |
| `src/snippet` | `compass-core` | Phase 5 | ✅ | 🟡 | ✅ | ❌ |
| `src/browser-extension` | — | **out of scope** | ✅ | n/a | n/a | never |

## Services

| C++ source | Rust home | Phase | C++ ✓ | Rust ✓ | parity test ✓ | C++ deleted ✓ |
|---|---|---|:-:|:-:|:-:|:-:|
| `src/services/app-runtime` | `compass-core` | Phase 1 | ✅ | 🟡 | ✅ | ❌ |
| `src/services/app-service` | `compass-core` | Phase 1 | ✅ | 🟡 | ✅ | ⏳ |
| `src/services/asset-resolver` | `compass-core` | Phase 1 | ✅ | ✅ | ✅ | ❌ |
| `src/services/audio-control` | `compass-core` | Phase 5 | ✅ | 🟡 | ✅ | ❌ |
| `src/services/autostart` | `—` | n/a (macOS) | ✅ | n/a | n/a | ❌ |
| `src/services/browser-extension` | — | **out of scope** | ✅ | n/a | n/a | never |
| `src/services/builtin-icon` | `compass-core` | Phase 1 | ✅ | ✅ | ✅ | ❌ |
| `src/services/calculator-service` | `compass-local-storage` | Phase 5 | ✅ | 🟡 | ✅ | ❌ |
| `src/services/clipboard` | `compass-clipboard` | Phase 3 | ✅ | 🟡 | 🟡 | ❌ |
| `src/services/desktop-notification` | `compass-notify` | Phase 5 | ✅ | 🟡 | ✅ | ❌ |
| `src/services/extension-boilerplate-generator` | `compass-core` | Phase 4 | ✅ | ✅ | ✅ | ❌ |
| `src/services/extension-registry` | `compass-core` | Phase 4 | ✅ | 🟡 | ✅ | ❌ |
| `src/services/extension-store` | `compass-core` | Phase 4 | ✅ | 🟡 | ✅ | ❌ |
| `src/services/file-chooser` | `compass-core` | Phase 2 | ✅ | 🟡 | ✅ | ❌ |
| `src/services/files-service` | `compass-xdg` | Phase 5 | ✅ | 🟡 | ✅ | ❌ |
| `src/services/font-service` | `compass-core` | Phase 5 | ✅ | ❌ | ❌ | ❌ |
| `src/services/global-shortcuts` | `compass-portals` | Phase 1 | ✅ | 🟡 | 🟡 | ❌ |
| `src/services/glyph-service` | `compass-core` | Phase 5 | ✅ | 🟡 | ✅ | ❌ |
| `src/services/image-fetcher` | `compass-core` | Phase 5 | ✅ | 🟡 | ✅ | ❌ |
| `src/services/input-server` | `compass-core` | Phase 5 | ✅ | 🟡 | ✅ | ❌ |
| `src/services/keybinding` | `compass-core` | Phase 5 | ✅ | ✅ | ✅ | ❌ |
| `src/services/local-storage` | `compass-local-storage` | Phase 4 | ✅ | ✅ | ✅ | ❌ |
| `src/services/media-control` | `compass-media` | Phase 5 | ✅ | 🟡 | ✅ | ❌ |
| `src/services/menu-bar` | `—` | n/a (macOS) | ✅ | n/a | n/a | ❌ |
| `src/services/navigation` | `compass-core` | Phase 2 | ✅ | ✅ | ✅ | ❌ |
| `src/services/news` | `compass-core` | Phase 5 | ✅ | 🟡 | ✅ | ❌ |
| `src/services/oauth` | `compass-oauth-store` | Phase 4 | ✅ | 🟡 | ✅ | ❌ |
| `src/services/paste` | `compass-core` | Phase 3 | ✅ | 🟡 | ✅ | ❌ |
| `src/services/permissions` | `—` | n/a (macOS) | ✅ | n/a | n/a | ❌ |
| `src/services/power-manager` | `compass-power` | Phase 5 | ✅ | 🟡 | ✅ | ❌ |
| `src/services/raycast` | `compass-core` | Phase 4 | ✅ | 🟡 | ✅ | ❌ |
| `src/services/root-item-manager` | `compass-core` | Phase 2 | ✅ | 🟡 | ✅ | ⏳ |
| `src/services/script-command` | `compass-core` | Phase 5 | ✅ | ❌ | ❌ | ❌ |
| `src/services/selection` | `compass-core` | Phase 3 | ✅ | 🟡 | ✅ | ❌ |
| `src/services/shortcut` | `compass-core` | Phase 5 | ✅ | 🟡 | ✅ | ❌ |
| `src/services/shortcut-inhibit` | `compass-core` | Phase 3 | ✅ | ❌ | ❌ | ❌ |
| `src/services/snippet` | `compass-core` | Phase 5 | ✅ | ❌ | ❌ | ❌ |
| `src/services/telemetry` | `compass-core` | Phase 5 | ✅ | 🟡 | ✅ | ❌ |
| `src/services/toast` | `compass-core` | Phase 4 | ✅ | ✅ | ✅ | ❌ |
| `src/services/tray` | `compass-core` | Phase 5 | ✅ | ❌ | ❌ | ❌ |
| `src/services/tray-host` | `compass-core` | Phase 5 | ✅ | ❌ | ❌ | ❌ |
| `src/services/update` | `compass-core` | Phase 5 | ✅ | 🟡 | ✅ | ❌ |
| `src/services/url-scheme` | `—` | n/a (Windows) | ✅ | n/a | n/a | ❌ |
| `src/services/wallpaper` | `compass-core` | Phase 5 | ✅ | ❌ | ❌ | ❌ |
| `src/services/window-manager` | `compass-core` | Phase 3 | ✅ | ❌ | ❌ | ❌ |
| `src/services/window-material` | `compass-core` | Phase 5 | ✅ | ❌ | ❌ | ❌ |

## Builtins

| C++ source | Rust home | Phase | C++ ✓ | Rust ✓ | parity test ✓ | C++ deleted ✓ |
|---|---|---|:-:|:-:|:-:|:-:|
| `src/builtins/browser` | — | **out of scope** | ✅ | n/a | n/a | never |
| `src/builtins/calculator` | `compass-core` | Phase 5 | ✅ | ❌ | ❌ | ❌ |
| `src/builtins/clipboard` | `compass-core` | Phase 5 | ✅ | ❌ | ❌ | ❌ |
| `src/builtins/developer` | `compass-core` | Phase 5 | ✅ | 🟡 | 🟡 | ❌ |
| `src/builtins/file` | `compass-core` | Phase 5 | ✅ | ❌ | ❌ | ❌ |
| `src/builtins/font` | `compass-core` | Phase 5 | ✅ | 🟡 | 🟡 | ❌ |
| `src/builtins/internal` | `compass-core` | Phase 5 | ✅ | ❌ | ❌ | ❌ |
| `src/builtins/media` | `compass-core` | Phase 5 | ✅ | ❌ | ❌ | ❌ |
| `src/builtins/power-management` | `compass-core` | Phase 5 | ✅ | 🟡 | 🟡 | ❌ |
| `src/builtins/raycast` | `compass-core` | Phase 5 | ✅ | ❌ | ❌ | ❌ |
| `src/builtins/root` | `compass-core` | Phase 5 | ✅ | ❌ | ❌ | ❌ |
| `src/builtins/shortcut` | `compass-core` | Phase 5 | ✅ | 🟡 | 🟡 | ❌ |
| `src/builtins/snippet` | `compass-core` | Phase 5 | ✅ | 🟡 | 🟡 | ❌ |
| `src/builtins/system` | `compass-core` | Phase 5 | ✅ | 🟡 | 🟡 | ❌ |
| `src/builtins/theme` | `compass-core` | Phase 5 | ✅ | 🟡 | 🟡 | ❌ |
| `src/builtins/vicinae` | `compass-core` | Phase 5 | ✅ | ❌ | ❌ | ❌ |
| `src/builtins/wm` | `compass-core` | Phase 5 | ✅ | ❌ | ❌ | ❌ |

## The window

`src/server/src/ui` — about **29,700 lines** across Qt Widgets and QML, and until this section
existed the ledger did not mention it. That was the largest omission in the file: 110 rows covered
every service, library and builtin, and none of them covered the thing a user actually looks at.
A row per subdirectory, with its C++ size, so that the distance is visible rather than implied.

| C++ source | lines | Rust home | Phase | C++ ✓ | Rust ✓ | parity test ✓ | C++ deleted ✓ |
|---|--:|---|---|:-:|:-:|:-:|:-:|
| `src/server/src/ui/qml` | 14,660 | `compass-ui` | Phase 5 | ✅ | ❌ | ❌ | ❌ |
| `src/server/src/ui/quick` | 3,806 | `compass-ui` | Phase 5 | ✅ | ❌ | ❌ | ❌ |
| `src/server/src/ui/views` | 2,760 | `compass-ui` | Phase 5 | ✅ | ❌ | ❌ | ❌ |
| `src/server/src/ui/settings` | 2,292 | `compass-ui` | Phase 5 | ✅ | ❌ | ❌ | ❌ |
| `src/server/src/ui/image` | 2,154 | `compass-ui` | Phase 5 | ✅ | ❌ | ❌ | ❌ |
| `src/server/src/ui/windows` | 1,881 | `compass-ui` | Phase 3 | ✅ | 🟡 | ❌ | ❌ |
| `src/server/src/ui/action-panel` | 1,366 | `compass-ui` | Phase 5 | ✅ | ❌ | ❌ | ❌ |
| `src/server/src/ui/bridges` | 539 | `compass-ui` | Phase 4 | ✅ | ❌ | ❌ | ❌ |
| `src/server/src/ui/alert` | 279 | `compass-core` | Phase 5 | ✅ | 🟡 | ✅ | ❌ |

The single 🟡 is `windows`, and it is generous: `compass-ui` opens one window, shows a text input
and a result list, moves a selection with the arrow keys, launches on Enter and dismisses on
Escape. That is the launcher's core loop and nothing else — no navigation stack, no action panel,
no views, no settings, no theming, no icons.

Three things about this section are worth stating plainly, because a table of ❌s invites the wrong
reading:

- **These rows are not all ports.** ADR-0001 chose Iced over Qt Widgets and QML, so most of this
  tree has no Rust counterpart to write — it has a *replacement* to design. `parity test ✓` on a
  QML row cannot mean a differential against the C++ widget; it will mean the ported behavioural
  tests §8.3 describes, or nothing.
- **`bridges` is Phase 4, not Phase 5**, because it is the seam the extension host renders through
  rather than chrome. `compass-extension-api` already models the view tree and its diff; what is
  missing is the half that turns a diff into pixels.
- **No line of this can be deleted before the Phase 7 cutover**, like every other row.

## Not-yet-ported scope

Tracked here so a 🟡 does not quietly become a ✅.

**`src/lib/xdgpp` → `compass-xdg`** — the desktop-entry, locale, value, reader and exec layers are
ported (47 C++ cases, verbatim inputs). Still C++-only:

- the `xdg-terminal-exec` draft extension (`X-TerminalArg*` typed accessors; the keys are readable
  through `Reader` today);
- the `DesktopFile` layer — `fromId`, `relativeId`, directory search. `from_file` and
  `ParseOptions::{id,path}` exist, but id computation and lookup are a separate pass;
- the sibling modules `bookmark`, `env`, `file-uri`, `file`, `mime`, `special`.

**`src/extension/api` → `compass-worker-host`** — the extension API's service adapters. `Storage`,
three of `OAuth`'s four, `UI/render`, `FileSearch/search` and all four `Clipboard` methods are
ported and pinned, as are all five `Application` methods, all four `Command` ones and all seven
`WindowManagement` ones, plus `Wallpaper/set` and both `BrowserExtension` methods: 33 of the 49
methods `figura/tsapi.fig` declares; with `UI`'s shell half that is 45 of 49. `UI/confirmAlert` joined them
once the host learned to hold a reply open: it is answered through `tsapi::Deferral` rather than by
a service returning a value, which is what a call that waits on a *person* needs. What is left is
`OAuth/authorize`, which needs a browser. Each sits behind a trait (`FileIndexer`, `Clipboard`, `Apps`,
`Commands`, `Windows`, `Wallpaper`, `Browser`, `Shell`) mirroring the C++'s own indirection, so the adapters are finished and tested while the backends they will call — the file
index, the Wayland clipboard, the launcher, the navigation and settings controllers — are still
ahead. A method not on `tsapi::IMPLEMENTED` answers with an error naming
itself rather than hanging the caller.

**`src/services/asset-resolver` → `compass-core::asset_resolver`** — ported **whole**, which is why
this row is green rather than amber: the base-path list, the first-match lookup, and the two
behaviours that make shared asset directories work — `addPath` does not deduplicate and `removePath`
erases one entry, so the first command to unload does not blind the second. The C++ singleton is an
ordinary value here; a singleton is how that file is reached, not what it does.

**`src/builtins/snippet` → `compass-core::snippet_form`** — the snippet form's validation and
`Expansion::validateKeyword`: a two-character minimum name, non-empty content with at most one
`{cursor}`, and a keyword that is optional but, when given, must be printable ASCII with no spaces
and at most 32 bytes. The two success toasts are not symmetrical ("Snippet updated" against
"Snippet successfully created") and are copied as they are. Still C++-only: the QML form, the
snippet store, and the manage-snippets list.

**`src/builtins/shortcut` → `compass-core::shortcut_form`** — the quicklink form: what each mode
prefills (`Copy of %1` only when duplicating, the quoted navigation titles), the reverts to
`default` when the saved app or icon no longer exists, the three required fields — link, app and
icon, but **not** the name — the `default` icon being stored as whatever it resolved to rather than
as the word, the favicon-over-opener rule for `http*` links, and the three link completions with the
cursor offset that lands inside `{argument name="|"}`. Duplicating takes the *create* path, as the
C++ does by branching on `Mode::Edit` alone. Still C++-only: the QML form, the favicon request, and
the manage-shortcuts list's action panel.

**`src/builtins/font` → `compass-core::font_browser`** — the grid model's decisions are ported:
the category dropdown (only categories some installed font belongs to, "All" at index 0, the
index-minus-one arithmetic, and the remembered choice that is restored only when it is not "All"),
the two headings with their counts, the search that scores the display name alone, the missing-glyph
placeholder and the colour-font rule that leaves an emoji font untinted, and the action panel whose
*primary* action is Preview rather than apply. The thirty-three-category table itself belongs to
`src/services/font-service`, which is its own row. Still C++-only: the grid widget and the specimen
view.

**`src/builtins/developer` → `compass-core::create_extension`** — the Create Extension form's
validation and what follows it: all six checks run every time so every mistake shows at once, the
description is held to 16 characters where the rest need 3, the location is the one check that asks
the filesystem, `expandPath` handles `~` and `~/` only (so `~root/x` is taken literally and fails),
and a success *replaces* the form on the navigation stack rather than stacking on it. Still
C++-only: the QML form itself, the boilerplate generator, and the success view's contents.

**`src/builtins/theme` → `compass-core::theme_picker`** — the list model and the view's own logic
are ported: the current/available split (and that the configured theme is filtered out like any
other when it does not match), the sort that only happens when something is typed, the name/
description weights with the id *not* searchable, the `Default theme description` fallback
subtitle, the eight palette swatches in the row's order, the action panel's two conditional
actions, and the live preview — selecting a row applies the theme and leaving the view puts the
configured one back. Still C++-only: the view host and the swatch rendering.

**`src/builtins/power-management` → `compass-core::power_commands`** — the catalogue and the run
plan are ported: eight commands in registration order with their titles, long descriptions and
keywords, the `confirm` preference (on for everything but Lock), the `customProgram` escape hatch
that exists only where a shell makes sense, and the two failure messages per command — whose
"can't" / "cannot" wording is inconsistent and stays that way, because these strings are
translated. The logind calls behind them are `compass-power`. Still C++-only: wiring the plan to a
confirmation dialog and a toast.

**`src/builtins/system` → `compass-core::browse_apps`** — the "Search Applications" builtin's
*model* is ported: the field weights (name 1.0, description 0.5, keywords 0.3 — **not** the root
list's 0.6), the `Hidden` accessory for a `NoDisplay` entry, and the action panel as data: focus the
first open window if there is one, open (clearing the search), each desktop action with
`control+shift+1..9` for the first nine only, then open-location behind the `action.open` keybind,
copy id, copy location. Still C++-only: the view host, the list widget, and the three other views in
that directory (`system-run`, `set-default-browser`, `set-default-terminal`).

**`src/services/shortcut` → `compass-core::shortcut`** — `Shortcut::parseLink`'s state machine and
`insertPlaceholder`'s argument rules are ported: literal text and placeholders in order, reserved
ids that expand on their own, `name=` / `default=` with and without quotes, and the two behaviours a
rewrite would "fix" by accident — a repeated key keeps its **first** value (`std::map::insert`), and
a link that ends inside a placeholder loses everything from the opening brace. Still C++-only: the
store behind it is ported too (`compass-core::shortcut_store`): the JSON file, the 10,000 limit,
the two different not-found sentences, the rollback when a write fails after the list already
changed, and the `value_or({})` that turns a corrupt file into an empty list rather than a refusal
to start. Still C++-only: the migration from the old `OmniDatabase`, and `resolveApp`.

**`src/services/app-service` → `compass-core::app_service`** — the lookups are ported:
`findById` (with its `.desktop` retry), `findByClass`, `find`'s id-then-class order,
`findCuratedOpeners`' dedupe by display name, and `list`'s case-insensitive sort. Still C++-only:
`findOpeners` / `findDefaultOpener`, which walk the MIME parent chain through `QMimeDatabase` — Rust
has no shared-mime-info reader here yet — and everything that starts a process (launch, the file
browser, the terminal), which belongs to whoever owns the session rather than to a lookup table.

**`src/services/root-item-manager` → `compass-core::root_items`** — the *search* is ported in
full: the weighted fields (title 1.0, subtitle 0.5, alias 1.0, keyword 0.6), the `MIN_QUALITY` gate,
the frecency boost, the empty-query `100 - FRECENCY_WEIGHT + FRECENCY_WEIGHT * frecency` ranking, the
enabled/provider/favourite filters, and the stable sort with its alias-prefix prioritisation. Twelve
tests, twelve controls, each read off `root-item-manager.cpp`. `mergeConfigWithMetadata`,
`registerVisit` and `resetRanking` are ported too — the enabled precedence (the item's own default,
then the user's per-item setting, then a *disabled* provider, which wins), aliases, shortcuts,
favourite positions and fallback flags. `searchGroupedByProvider` is ported
too, with its two rules that differ from the flat search — a provider whose *display name* matches
contributes all of its items, including ones scoring zero, and `providerId` is not applied — for
another nine tests and nine controls. Still C++-only: the manager around it — loading items from
providers, `mergeConfigWithMetadata`, recording a visit, and `setAlias` / `setProviderEnabled` with
the config writes behind them.

**`vendor/sqlcipher` + `vendor/fuzzy-trigram` → `compass-sqlcipher-sys`** — the storage engine
itself, built from the same C the C++ engine links (ADR-0014). `Database::open` does what
`ClipboardDatabase`'s constructor does and in the same order — open, key with raw bytes in
SQLCipher's `x'...'` form, register `fuzzy_trigram`, apply the four pragmas — because that order is
load-bearing. Tested against real encrypted files rather than SQL strings: the file has no
`SQLite format 3` magic and does not contain its own payload in the clear, a wrong key is refused by
`open`, the FTS table the clipboard schema declares can be created and queried, and a second
connection to an existing encrypted file still has the tokenizer. Not yet ported: blobs, the
transaction wrapper, and `sqlite3_changes` (deliberately — see `tryBubbleUpSelection` below).

**`src/services/clipboard` → `compass-clipboard`** — **`clipboard-db.cpp` (478 lines) is ported in
full.** Every function `clipboard-db.hpp` declares has a Rust counterpart:

| C++ | Rust |
|---|---|
| `searchTerms` + the trigram predicate | `search::plan` |
| `runMigrations` | `schema::run` |
| `query` | `store::query` |
| `insertSelection`, `insertOffer`, `indexSelectionContent` | `write::insert_selection`, `insert_offer`, `index_content` |
| `removeSelection`, `removeAll`, `evictOlderThan` | `write::remove_selection`, `remove_all`, `evict_older_than` |
| `tryBubbleUpSelection` | `write::bubble_up` |
| `setPinned`, `setKeywords`, `retrieveKeywords` | `write::set_pinned`, `set_keywords`, `keywords_of` |
| `oldestEvictableTimestamp` | `write::oldest_evictable` |
| `findSelection`, `findPreferredOffer` | `write::find_selection`, `find_preferred_offer` |

It runs on `compass-sqlcipher-sys`, which builds `vendor/sqlcipher` and `vendor/fuzzy-trigram` from
the same C the C++ engine links ([ADR-0014](./adr/0014-clipboard-storage-is-sqlcipher-plus-a-vendored-tokenizer.md)),
so both engines read and write the same encrypted files with the same tokenizer.

**Still C++-only, and the reason the row is 🟡:** `clipboard-service.cpp` — the Wayland selection
watcher, payload storage on disk, and the encryption of those payloads. That is the layer *above*
the database, and it is the caller that unlinks the blobs whose ids `evict_older_than` now reliably
returns.

Four C++ bugs are fixed rather than reproduced, each pinned by a control that fails when the
original shape is put back: the eviction blob leak, `tryBubbleUpSelection` answering from a
connection-wide counter, the migration checksum that was written and never compared, and
`runMigrations` swallowing its own failures. All four are in the divergences table below.

**Why `parity test ✓` is 🟡 rather than ✅.** `vicinae::fuzzy` and `vicinae::crypto` each compile
standalone, which is what lets CI diff the real C++ implementation against ours. `clipboard-db.cpp`
does not — it pulls in Qt, `db::Database` and `MigrationManager` — so there is no C++ binary to
diff against. What exists instead: every assertion is shown to fail against a deliberately wrong
port, and the tests run against real SQLCipher files rather than SQL strings. That is strong
evidence and it is not a differential, which is what the amber says.

The gap worth naming is `QString` iterating UTF-16 code units, so a non-BMP character counts as
*two* word characters: `"a😀"` is a trigram run to the C++ engine and would not be to a port walking
Rust `char`s. `has_trigram_run` walks `encode_utf16` for that reason, and
`a_non_bmp_character_is_two_word_characters_not_one` holds the scalar-walking version alongside it
so the case is shown to discriminate. The same trap recurs in `index_content`, where
`QString::left(65536)` can split a surrogate pair.

## Out of scope for the port

Rows marked **out of scope** are not "not yet" — the Rust engine will never implement them.

- **Browser tab search and switching** (`src/browser-extension`, `src/services/browser-extension`,
  `src/builtins/browser`) becomes an extension rather than a builtin, per
  [ADR-0008](./adr/0008-browser-control-is-an-extension.md). The C++ implementation keeps working
  for C++-engine users and is not deleted by the port. If no extension exists by Phase 7, this is a
  feature regression at cutover and must be in the release notes.

  Note for whoever builds it: native messaging does not simply move into a TypeScript extension. A
  browser spawns a native host **binary by absolute path** from a manifest in a fixed location, and
  a Raycast-compatible extension is a TS bundle spawned by the launcher with no such path — inside a
  Flatpak on our first target. ADR-0008 sets out the three ways round that and deliberately does not
  pick one.

## Deprioritised

**AppImage — disabled, not deleted.** Not a distribution channel we plan to use, so the AppImage
build is not being chased. Its jobs could only ever fail: `ghcr.io/tuna-os/compass/build-env:appimage-*`
was never published after the move off Depot, and nobody is going to publish it.

Rather than accept a permanently red check — which is how a team learns to ignore CI — the automatic
triggers are gone:

| Where | State |
|---|---|
| `build-appimage.yaml` | `workflow_dispatch` only; `push`/`pull_request` removed |
| `build-appimage-image.yaml` | `workflow_dispatch` only; weekly `schedule` and `push` removed |
| `build-appimage` job in `release.yml` | `if: false`, and dropped from the `needs:` of `publish-npm` and `publish-release` |
| `scripts/runners/appimage/` | untouched |

That last row mattered more than the others. Both publish jobs listed `build-appimage` in `needs:`,
so its failure skipped them — **releases were already impossible**, not merely missing an AppImage.
Ungating them is a fix, not a regression; the cost is that a release no longer ships a `.AppImage`,
which is the intent rather than a side effect.

To restore: publish the build-env image by dispatching `build-appimage-image.yaml` once (it compiles
GCC 15.2 and Qt 6.10 from source, so expect hours), then restore the triggers and the `needs:`
entries. Restoring the triggers alone just brings the red back.

Bluefin is the first target and is Flatpak-only, so the Flatpak manifest in `packaging/flatpak/` is
the channel that matters.

## Declared divergences

Behaviour that intentionally differs from the C++ engine. Each is pinned by a test that fails if
the behaviour changes, so a future fix is loud rather than silent.

### `compass-clipboard` — two C++ behaviours deliberately **not** reproduced

| # | C++ behaviour | What we do | Pinned by |
|---|---|---|---|
| -2 | `evictOlderThan` computes its cutoff with `unixepoch()` in **both** the `SELECT` that collects the offer ids to unlink from disk and the `DELETE` that removes the rows. Those are separate statements with separate readings of the clock (measured: inside `BEGIN`, `unixepoch('subsec')` advanced after 434 consecutive statements), so the `DELETE` set is a superset and anything crossing the threshold in between is deleted but never reported — its payload stays on disk forever. | Compute the cutoff once and bind it to both statements, which makes the two sets identical by construction. | `eviction_returns_every_offer_it_deletes` (fails when the second reading is reintroduced) |
| -1 | `tryBubbleUpSelection` runs its `UPDATE`, discards whether it succeeded, and answers from `m_db.changes()` — a connection-wide counter holding the most recent *successful* statement's count. A failed or no-op update can therefore report success, and its caller (`clipboard-service.cpp:508`) then skips `insertSelection`, so the copied content never reaches the history. | `RETURNING id`: did *this* statement touch a row. `compass-sqlcipher-sys` deliberately does not expose `sqlite3_changes`. | `bubbling_up_something_absent_reports_false_even_after_a_successful_write` |
| 0 | `query` divides by `limit` to compute `totalPages` (`ceil(totalCount / limit)`), so a zero `limit` is a division by zero whose result is cast to `int`. It also interpolates `limit` and `offset` into the SQL text with `.arg()` rather than binding them. | Refuse a non-positive `limit`; bind both. `current_page`'s ceiling rounding *is* reproduced, oddity included — it is a display value the C++ UI already agrees with. | `a_zero_limit_is_refused_rather_than_dividing_by_it` |
| 1 | `MigrationManager::runMigrations` catches every exception, logs it, rolls back and returns `void`; `ClipboardDatabase::runMigrations` returns `void` too. A failed migration is silent, and the next thing the user sees is every query failing against a schema that was never created. | `schema::run` returns a `Result`. | `an_edited_migration_is_refused`, `a_database_from_a_newer_build_is_refused` |
| 2 | The `checksum` column exists to detect a migration edited after it was applied. `insertMigration` writes it and `loadDatabaseMigrations` reads it back into a struct field — and nothing ever compares the two. It is a stored value with no reader, so the detection it exists for never happens. | Compare it, and refuse on a mismatch. The expected hashes are also pinned in `schema.rs`'s tests, so editing a migration fails at development time rather than on a user's machine. | `an_edited_migration_is_refused`, `the_embedded_content_hashes_to_what_the_cpp_engine_recorded` |

### `compass-xdg` — six C++ bugs deliberately **not** reproduced

| # | C++ behaviour | What we do | Pinned by |
|---|---|---|---|
| 1 | `parseRawLocale` loops `while (!isPeek(']'))`, and `isPeek` is false at EOF because `peek()` returns `0`. A truncated `Name[fr` at end of file appends NUL forever: an unbounded loop with unbounded allocation. | Terminate on `]`, newline, or EOF. | `a_truncated_locale_suffix_does_not_swallow_the_next_line` |
| 2 | A key with no separator consumes the following character and, on mismatch, skips to the *next* newline — so when the consumed character *was* the newline, the next line is silently swallowed. | Skip only when the mismatched character is not the line terminator. | `a_key_with_no_separator_is_skipped` |
| 3 | Field codes push directly into `args` while the in-progress word sits in `part`, so `Exec=prog --name=%c` yields `["prog", "MyFile", "--name="]` — the pending word lands *after* the expansion. `--file=%f` forms are common in the wild. | Substitute single-valued codes (`%f %u %c %k`) into the current word; flush the word before multi-valued ones (`%F %U %i`). Every C++ test result is unchanged, since they all use standalone codes. | `a_field_code_may_be_glued_to_the_rest_of_a_word` |
| 4 | `asStringList` unescapes each element at a separator but pushes the trailing residue raw, so `Keywords=a;b\sc` yields a literal `b\sc`. | Unescape consistently. | `escapes_apply_to_an_unterminated_last_element` |
| 5 | An unknown `Type` silently becomes `Application` (the if/else chain has no `else`). | `EntryType::Other(String)`; `is_application()` is false for `Type=ServiceType`. A *missing* `Type` still defaults to Application, matching C++. | ported `Type` cases |
| 6 | `genericName()`, `version()` and `unlocalizedName()` return `std::optional` built from a plain `std::string`, so they are never `nullopt` — an absent key reads as `Some("")`. | Return `None`. | `unlocalized_name_is_absent_when_only_a_localized_name_exists` |

Matched deliberately, for the record: field codes are not expanded inside quotes; unknown and
deprecated field codes expand to nothing; a redeclared group replaces rather than merges; localized
score ties resolve to the last declaration.

### `compass-core::default_app` — one picker sorts stably and the other does not

`set-default-browser-view-host.hpp` sorts its candidates with `std::ranges::stable_sort`;
`set-default-terminal-view-host.hpp`, which is otherwise a copy of it, uses `std::ranges::sort`.
Both comparators are `isDefault(a) > isDefault(b)` and say nothing about two non-defaults, so the
terminal list below the current default is in an unspecified order — two runs of the same binary may
disagree, and nothing in the view depends on it being one way or the other.

Compass sorts both stably, so the rest keep the order the application database gave them.
`the_rest_keep_the_order_the_database_gave_them` pins it for the browser picker, where the C++ makes
the same promise; the terminal picker shares the implementation, so it inherits a guarantee the C++
does not make rather than a different behaviour.

### `compass-core::root_items` — an unfavourited item that stays unfavourited

`mergeConfigWithMetadata` only *assigns* `favoriteIdx` when the entrypoint id is in `cfg.favorites`:

```cpp
if (auto it = std::ranges::find(cfg.favorites, std::string{entrypointId}); it != cfg.favorites.end()) {
  meta.favoriteIdx = std::distance(cfg.favorites.begin(), it);
}
```

`m_metadata` is a map that outlives the merge, so removing an item from favourites leaves the old
index in place. Every search that passes `includeFavorites = false` — the root list, which renders
favourites separately — keeps dropping that item until the launcher restarts. `meta.fallback` two
lines below is assigned unconditionally and does not have the bug.

Compass clears the index when the id is absent. `unfavouriting_clears_the_index_rather_than_leaving_it_behind`
pins the fixed behaviour; `a_fallback_that_is_removed_stops_being_one` pins the neighbouring line
that was already right, so a future "fix" that changed the wrong one would be caught.

### `compass-worker-host::ui_shell_service` — a method that does nothing, faithfully

`ExtUIService::updateToast` is `{ return Void::ok(); }`. The body is empty: an extension that calls
`toast.title = "..."` after showing a toast gets a resolved promise and no change on screen. The
Rust host does the same, and `updating_a_toast_does_nothing_at_all_because_the_cpp_does_nothing`
pins it — with a control that fires if it ever starts working.

Implementing it would be the more useful behaviour and the wrong port: the extension cannot tell
from the reply which host it is talking to, so a Compass that updated the toast would show text a
Vicinae user never sees, and an extension author would tune their toasts against the wrong one. When
the C++ grows a body, this test is the one that should fail.

### `compass-core::slug` — two regex passes that reach the same string as one

Qt's `slugify` replaces `[\s_]+` with the separator, which collapses a run of whitespace in that
one pass, and then collapses runs of the separator in a later pass. The Rust port writes one
separator per whitespace character and lets the later collapse do both jobs. It also drops the
C++'s early return on an empty input, which cannot change the result because an empty string falls
through every remaining step unchanged.

This is recorded rather than silently done because it was found by a control that did not fire:
mutating the whitespace-run logic changed nothing observable, because the collapse pass rescued it.
A behaviour guarded twice is a behaviour whose guard cannot be tested, so the redundant guard went.
`a_run_of_whitespace_makes_one_separator_not_many` still pins the property, and now fails when the
single remaining rule is broken.

### `compass-core::boilerplate` — two commands with the same slug, and the first one wins

`QFile::copy` refuses to overwrite an existing destination and reports the failure through a return
value the C++ does not check. So an extension generated with two commands whose titles slugify alike
— "Show Things" and "show things" — gets two entries in its manifest and one source file, holding
the *first* command's template. The second command points at a file that is not the template it
asked for.

The port reproduces this, and `two_commands_that_slugify_alike_do_not_clobber_each_other` pins it,
because the alternative readings are both worse: overwriting would make the *second* command win and
leave the first pointing at the wrong template instead, and rejecting the config outright would fail
a generation the C++ completes. It is a bug worth fixing upstream, not worth diverging on here — the
test is named for what it protects, and is the one that should fail when the C++ starts checking
that return value.

### `src/file-indexer` stays ❌ although part of it is ported

`compass-core::entry_filter` is a complete port of `entry-filter.cpp` — the rules deciding which
directory entries the indexer walks into — and `compass-core::query_policy` of
`file-indexer-query-policy.cpp`, which decides what a typed query asks the index. Between them, 70
tests and 46 controls. The row stays ❌ anyway.

It covers 5,646 lines across fifteen files: the SQLite schema and its writer, the query engine and
its policy, the incremental scanner, the scan dispatcher, the filesystem walker and the watchers.
Two files of those fifteen are not the row, and marking it 🟡 would put a colour on this ledger that
means "a model landed without its backend" when what actually happened is "a fifteenth of the row
landed". The percentage in PLAN.md is only worth anything if a row's colour means one thing.

What the ports are worth is not in the score. An indexer that walks `/proc` never finishes, and one
that walks `~/.cargo/registry` fills the index with vendored sources that rank above the file
somebody wanted. Those rules are a long list of specific names rather than a general principle, and
a name quietly dropped from the list is not a bug anyone reports — it is a search that stops being
useful. So the list is now pinned, whatever colour the row is.

The query policy is the other half of that argument. Splitting and quoting a query is mechanical;
deciding *which* misspellings to try is not. Too few and a typo finds nothing, too many and the
index is asked several questions per keystroke, each ranking worse than the one the person meant.
None of that fails anything — it makes search feel slightly unreliable. Every rule now has a test
naming what it prevents: why a word the corpus knows well is left alone, why only one word per
family is tried and why the shorter one wins it, and why every plan after the first differs from it
in exactly one word rather than enumerating combinations nobody typed.

Two C++ behaviours it reproduces rather than fixes, both in `GitIgnoreReader`: only the *filename* is
matched against a pattern, so `build/*.o` never matches anything, and a leading `/` is stripped
rather than anchoring, so `/target` matches a `target` anywhere in the tree. The C++ calls this a
hack in its own comment. Narrowing it would start indexing directories people's `.gitignore` files
currently keep out; widening it would drop files they expect to find. Every line of an ignore file
becomes a pattern too, comments included — a `#comment` glob never matches anything real, which is
why nobody has noticed.

### `compass-core::raycast_store` — the same escaping fix, and a filter deliberately skipped

`search` and `fetchExtension` build their URLs with `QString::arg` exactly as the Vicinae store's do,
so the same divergence applies and for the same reason: an extension name containing a `/` would
otherwise add a path segment and ask the API for something else.

What is *not* changed is the platform filter being skipped on Linux entirely. Raycast is a macOS
product and its extensions advertise `macos`; filtering on that here would produce an empty store
rather than a best-effort one, which is what the C++ comment says. The compatibility sheet is the
other half of the bargain — fetched only on the platform where the filter was skipped, and saying
which of those extensions actually work. `the_compat_sheet_exists_only_where_the_filter_was_skipped`
pins that the two conditions are exact opposites, so a platform can never both skip the filter and
have nothing to consult.

A failed compat fetch is not an error: the C++ warns, returns an empty map, and leaves its
`m_compatFetched` flag false, so the store keeps working without notes and the next request tries
again. Both halves are reproduced.

### `compass-core::extension_store` — a search query that is actually escaped

`VicinaeStoreService::search` builds its URL with
`QString("/store/search?q=%1").arg(query)`, which substitutes the query verbatim. A query containing
`&`, `#` or `=` therefore changes the *shape* of the URL rather than the value of `q`: searching the
store for `a & b` asks the server for `q=a ` plus a parameter called ` b`, and searching for `c#`
sends `q=c` with a fragment.

The port percent-encodes the value. Everything unreserved is left alone, so an ordinary search
produces byte-identical output to the C++ and only the queries that were already broken change.
`an_ordinary_search_makes_the_url_it_always_made` pins the first half and
`a_search_containing_an_ampersand_stays_one_parameter` the second.

### `compass-core::semver` — an overflowing component is refused rather than wrapped

`Semver::parse` accumulates into an `unsigned` with `current * 10 + digit` and no overflow check, so
a component past 2^32 wraps. `4294967296.0.0` therefore compares equal to `0.0.0`, and a release
tagged that way would look like no release at all. The port returns `None`, which makes the tag "not
a release tag" — a refusal the update service already knows how to ignore — rather than a wrong
answer it would act on.

What is *not* changed is that `Semver` is not semver. It parses a dotted run of decimal integers and
nothing else: `v1.2.3-rc1` does not parse. That is load-bearing, because it is what keeps release
candidates from being offered as updates without anyone having to filter them, and
`a_prerelease_tag_does_not_parse_at_all` says so where someone might otherwise "fix" it.

One bug found while porting, in the port rather than the C++: deriving `PartialEq` compares the
component lists structurally, which would make `1.0` and `1.0.0` unequal *and* neither greater — a
contradiction a sort or a hash map can act on. The C++ defines `operator==` in terms of `<=>`; the
port now does the same, and hashes on the components with trailing zeroes removed so `Hash` agrees
with `Eq`. `equality_agrees_with_the_comparison` pins it.

### `compass-core::telemetry` — two C++ bugs deliberately **not** reproduced

`TelemetryService::setEnabled` remembers its previous value in a *function-local `static`*. That
makes the flag process-wide rather than per-instance: it is shared by every `TelemetryService` and
survives one being destroyed. With a single service per process this is invisible, which is
presumably why it has lasted. With two, the second one's first `setEnabled(true)` is swallowed as
"no change" and its telemetry silently never starts. The port keeps the flag on the instance, which
is what the code reads as though it did.
`each_service_keeps_its_own_enabled_flag` pins it.

`loadState` warns on an unreadable state file and carries on with whatever glaze left in `m_state` —
for a parse failure that is a default-constructed `State`, so `userId` is the empty string and every
record from then on is filed under `""`. Not a crash, and not visible locally: it just quietly
detaches that machine's records from each other. The port generates a fresh id instead and writes it
back, so the records stay attributable to *a* machine and the next run is stable again.
`a_corrupt_state_file_gets_a_fresh_id_rather_than_an_empty_one` pins both halves.

Neither divergence changes what is collected, only whether it is coherent. What *is* reproduced
exactly is the record's shape: glaze serialises C++ member names as written, so the wire keys are
`userId`, `vicinaeVersion`, `systemInfoLastSentAt` and the rest in camelCase, and the port renames
its snake_case fields to match. Also reproduced is which fields are lowercased — `architecture`,
`buildProvenance`, `vicinaeVersion` and each entry of `desktops`, and not the other seven. That
asymmetry looks accidental, but normalising the rest would make this engine's records group
differently from the C++'s in the same dataset.

### `compass-core::paste` — a copy that happens even when the paste cannot

`PasteService::pasteContent` calls `copyContent` first and only then asks whether the platform
supports pasting. On a platform that does not, the content is on the clipboard and the caller is
told `false`.

Reproduced rather than tidied. It is arguably the more useful outcome — the person can paste it
themselves — and reordering would break a caller that retried on `false`, which would then copy
twice. `a_platform_that_cannot_paste_still_gets_the_copy` pins both halves: the copy happened, and
nothing was scheduled.

The timeout path is the same kind of thing and is pinned the same way: when focus never lands the
paste is dropped and the clipboard is *not* restored, so what was copied is still there. A port that
helpfully restored it would take away the only consolation prize the failure has.

### `compass-core::audio_control` — a volume that is not a number no longer takes the process down

`toAudioSink` reads a channel's volume with `std::stod(percent)`, which parses the leading number
and ignores the `%`. On a string with no leading number it does not return anything — it throws
`std::invalid_argument`, from inside a function with no `try` anywhere above it. A `pactl` that
printed `"n/a"` for a channel would end the process.

The port reads the leading numeric run and falls back to 0.0, which is what the sink already reports
when its channel map is empty, so the failure mode is "this sink reads as silent" rather than
"Compass exited". `a_volume_that_is_not_a_number_reads_as_zero_rather_than_crashing` pins it.

The lexicographic channel rule *is* reproduced: the C++ takes `volume.begin()->second` from a
`std::map`, so a stereo sink reports its `front-left` level, and this port uses a `BTreeMap` to get
the same answer. Two controls pin it — taking the last channel instead, and averaging — both of
which fail the suite. (Swapping the `BTreeMap` for a `HashMap` does *not* reliably fail it, which is
a defect in that mutation rather than in the test: it makes the result vary per process instead of
being wrong in a fixed way, so a suite that passed once proves nothing either way.)

### `compass-core::alert` — a replaced alert is a cancelled alert, and the caller has to be told

`AlertModel::handleAlertRequested` calls `triggerCancel()` on the alert already showing before it
takes the new one. The TypeScript API documents the consequence — "Calling this function when
another alert is currently pending will result in the pending alert to be automatically canceled" —
and it matters because the cancelled alert is some extension's un-settled promise.

So [`AlertModel::show`] returns the previous alert's resolution rather than dropping it. That is a
shape change from the C++, where the resolution goes out through a callback the widget owns, and it
is deliberate: a caller that ignores a `#[must_use]`-shaped return is a caller a compiler can
complain about, where a caller that forgets to connect a signal is not.

The port also keeps the C++'s routing of *every* exit that is not the confirm button — cancel,
navigating away, being replaced — through one place that answers `false`. A fifth exit added later
should have to opt in to `true` rather than out of it.

### `compass-platform-linux::keyboard` — four modifiers that are declared and ignored

`UInputKeyboard::Modifier` names six modifiers. `applyMods` and `clearMods` test two of them. A
caller passing `Alt`, `Logo`, `Altgr` or `Capslock` gets a bare keystroke with no modifier held —
and still gets the *slow* path, because the C++ chooses its delay on `mods ? ... : ...`, the raw
integer, before deciding what to do with it. So an ignored modifier costs 10ms per key and changes
nothing.

Both halves are reproduced and pinned. Making Alt work would be the obvious fix and the wrong port:
a snippet bound to Alt+F would send a keystroke on Compass that it does not send on Vicinae, with no
way for the caller to tell which build it is on. The tests are named for what they protect, and are
the ones that should fail when the C++ grows the other four branches.

Two trailing `SYN_REPORT`s in `sendKey(code, mods)` are likewise redundant — `sendKey(code)` already
ends with one — and likewise kept, with `a_shifted_keystroke_is_exactly_this_sequence` pinning the
whole wire in order.

### `compass-core::fetch_queue` — an abort that frees a slot without filling it

`NetworkFetcher`'s abort handler erases the reply from the in-flight map and emits
`abortRequested`. It does not call `startRequests`. So aborting an in-flight image fetch leaves one
of the six slots empty until some *other* request finishes, and a queued request that could have
taken it waits instead.

The port reproduces this, and `aborting_an_in_flight_request_does_not_start_the_next_one` pins it
with a control that fires the moment the slot is refilled. Calling `start_requests` there is the
better scheduler and the wrong port: a person scrolling a list of remote icons fast enough to
cancel requests would see Compass issue a different number of them than Vicinae, at different
times, and any comparison of the two under load would be measuring this difference rather than the
thing being compared. `the_slot_an_abort_freed_is_taken_by_the_next_completion` pins the other half
— the queue is delayed, not wedged.

### `compass-core::root_items` — a hash order made deterministic

`searchGroupedByProvider` buckets into a `std::unordered_map<std::string, Bucket>` and then stable-sorts
the groups by score. Stable sort preserves the order it was given, and the order it is given is the
map's iteration order — a hash order over the provider ids present. Two groups that score equally
therefore come out in an order that depends on which other providers matched, and can change between
builds of the same binary.

Compass buckets in first-appearance order instead: the first item belonging to a provider creates its
group, so a tie resolves to the order the items were registered in. `groups_that_tie_keep_first_appearance_order`
pins it. There is no "fails if the C++ is fixed" test to pair with this one, because the C++ is not
wrong in a way a test can name — it is unspecified, and this is a choice within it.

### `compass-power` — one C++ bug deliberately **not** reproduced

| # | C++ behaviour | What we do | Pinned by |
|---|---|---|---|
| 1 | `SystemdPowerManager::can` calls `CanPowerOff`/`CanSuspend`/`CanHibernate`/`CanReboot` and then answers `!reply.arguments().isEmpty()` — it never reads the reply. logind answers with a *string*: `"yes"`, `"no"`, `"challenge"` or `"na"`. All four are a non-empty argument list, so a machine that cannot hibernate is offered Hibernate, and the menu entry does nothing. | Read the string. `Capability::is_offerable` is true for `yes` and `challenge` (polkit will ask), false for `no`, `na` and anything this build does not recognise. | `logind_replies_are_read_rather_than_counted`, `the_capability_reply_is_read_and_not_merely_counted` (drives a real reply through a mock logind), and `the_cpp_still_has_the_bug_this_port_declines_to_copy`, which fails if the C++ is fixed |

### `compass-crypto` — one error variant the C++ API cannot express

Not a behavioural divergence; a faithful reproduction of an awkward C++ signature, recorded so the
next person does not "fix" it on one side only.

`Crypto::AES256GCM::EncryptError` has exactly one value, `CipherError`, so C++ `encrypt` reports a
wrong-sized key, an unavailable CSPRNG and a refusing cipher identically. `decrypt`'s error set, by
contrast, *does* distinguish `InvalidKeySize`. `compass-crypto` mirrors both, including the
asymmetry, because the parity harness compares error names and a more precise Rust variant would
read as a divergence rather than as the improvement it is. Fixing it means changing both sides in
the same commit.

The reverse case is also worth naming: C++ `DecryptError` declares a `CipherError` variant that
**no code path produces** — `gcmDecrypt` failure maps to `AuthFailed`. So the reachable C++ set is
`{InvalidKeySize, DataTooShort, AuthFailed}`, which is exactly the Rust set. `aes-gcm` collapses
every decryption failure into one opaque error by design, and that is the right call: telling
"the cipher broke" apart from "the tag did not verify" is a padding-oracle-shaped invitation.

### `compass-search` — nucleo is not fzf

`nucleo-matcher` uses a different algorithm from the C++ fzf port, so absolute scores are on another
scale and are never asserted. The normalized 0-100 `score`/`quality` values match the C++
expectations closely: every `score == 100`, `score == 50` and `quality == 100` assertion ports
verbatim and passes. What does not:

| # | Divergence | Impact | Pinned by |
|---|---|---|---|
| 1 | ~~No coherence signal~~ — **resolved.** The original claim here was wrong: the C++ backtracker only ever compares `B[j]` against *zero*, never uses its magnitude, and uses `H`/`C` solely to choose the alignment. So `coherent = !boundary_inside \|\| !mid_word_run_start` is a pure function of (haystack, indices), which is exactly what nucleo returns. It is an **exact port**, not a heuristic — no thresholds exist to tune. | None | the ported coherence suite |
| 2 | **One ordering flip** (upstream issue #946). For `"Spo"`, C++ gives `Spotify > Reload Script Directories > Sysprog`; we give `Spotify > Sysprog > Reload Script Directories`. nucleo prefers a short scatter inside one word starting at position 0; fzf's larger word-boundary bonuses pull the other way. Every other ordering case ports and passes. | Low | `diverges_spo_ordering` |
| 3 | **Narrower diacritic folding.** nucleo folds precomposed accents (é, ñ, ü) but not Latin Extended-A stroked/ogonek letters (Ł, ź, đ, ż), so `"lodz"` does not match `"Łódź Express"`. fzf's table covers them. | Affects Polish, Czech and Croatian app names | `diverges_latin_extended_a_is_not_folded` |
| 4 | **Ties that discriminate nothing.** For `"clip"`, all of `Clipboard History`, `Clear Current Clipboard Data` and `Clear Clipboard History` score identically, because nucleo's score depends only on the matched region, not on haystack length or match position. The C++ ordering test passes there only because `stable_sort` preserves input order — so that case discriminates nothing in *either* implementation. | Real discrimination needs a length or match-position penalty layered on top of nucleo | `diverges_clip_ordering_is_a_three_way_tie` |
| 5 | **Char, not byte, offsets** — a deliberate API change. `"Café Bar"`/`"bar"` reports `5..8` where C++ asserts bytes `6..9`. | None; byte offsets are recoverable | the range tests |

### How closely is "closely"? 79.2%

The table above was written from a ported ordering suite over hand-written cases, which could say
*that* nucleo and fzf differ but not *how much*. `compass-testkit`'s `scorer-parity` bin now
measures it directly, against the real C++ scorer compiled from `src/lib/fuzzy` — that library is
header-only with no Qt dependency, so it costs one translation unit.

Over 738 harvested entries and 1685 queries derived from them:

| | |
|---|---|
| identical | 1333 (79.2%) |
| divergent queries | 352 |
| divergent (query, entry) pairs | 1417 |

| shape | count |
|---|---|
| C++ rejected, Rust accepted | 771 |
| both accepted, C++ higher | 440 |
| both accepted, **Rust** higher | 145 |
| **Rust** rejected, C++ accepted | 61 |

Both directions occur, which is what two different algorithms produce and what a smaller corpus
hid: over the previous 115-entry set only six queries diverged and every one had C++ stricter.

The divergence is entirely in the **raw matcher** — both `score_query` implementations normalise
identically. Matches at index 0 and fully contiguous matches agree exactly; `a3`/`3` scores 30
against 26 (divergence #4, position is not scored), and `System`/`Se` scores 29 against 47
(divergence #2, the word-boundary bonus).

**But rank agreement is near-total.** Score equality is not what a user sees; over the same 1685
queries the two engines pick the **same top result 100% of the time** — including all 920 queries
that return more than one hit, so it is not the trivial single-candidate kind — the same top 3 in
97.2%, and the same full order in 84.4%. The scoring difference is real and almost entirely
absorbed by the ranking. `scorer-parity` therefore *asserts* top-1 agreement exactly, rather than
ratcheting it.

`scorer-parity` pins the score totals as a ratchet: it fails if they get worse **and** if they get
better, so a nucleo bump cannot move what users see without someone looking at it. Driving them to
zero would mean replacing nucleo, which PLAN §10 settles the other way.

### The residual coherence gap, one level up

Coherence itself is now exact, but it is a property *of an alignment*, and nucleo's DP does not
always pick the same alignment fzf does. Alignments differ in about 10.6% of cases; validated
against a C++ oracle, end-to-end agreement on the `coherent` flag is **99.57%** (3032/3045), and
**100% across the entire ported corpus and every `main.cpp` ranking case**. Every disagreement is an
alignment difference, e.g. `"Settings System"/"stem"`: C++ aligns `S(0) t(2) e(13) m(14)` and calls
it incoherent, nucleo aligns `Sys[tem]` and calls it coherent — arguably the better read.

Closing that would mean replacing nucleo's DP, not improving the classifier. Since it has no
observable instance in the corpus, it is pinned by `diverges_coherence_depends_on_nucleos_alignment`
(which also freezes nucleo's alignment, so a future nucleo bump surfaces the change) rather than
carried as a behavioural divergence.

### A recall cost that arrives with parity

Inherited from the C++ rule, not introduced by the port, and worth knowing before anyone reports it
as a regression: cross-word abbreviations that pick up a mid-word letter are rejected.
`"Firefox Web Browser"/"ffb"`, `"Text Editor"/"txted"` and `"Power Statistics"/"pwrstat"` are all
rejected — and the oracle confirms the C++ rejects all three too. The rule is working as designed;
whether it is the *right* design is a separate product question.
