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
| `compass-core` | 1,469 | app index, frecency, config, root search, glyphs, snippets, toasts, quicklinks, the extension boilerplate generator, the image fetch queue, the confirm dialog, volume and mute, the paste handoff, the telemetry record, update checks, the news notices, the selected text, the file dialog, both extension stores, emoji metadata, the snippet input server's framing, the icon URL scheme, contrast colours, the two per-window Wayland registries, six desktops' wallpaper vocabularies, the font browser's grouping, snippet expansion, the tray menu and the StatusNotifierItem host, the script-command scan, the calculator history view, the window and workspace switchers, the media and volume commands, the file search command, the Markdown showcase, the Raycast store views, the root list's clock and shortcuts, the emoji picker's skin tones, the bug report and fallback manager, the window-manager dispatch and focus memory, the indexer's entry filter, query policy and result ranking |
| `vicinae` | 184 | CLI, an 11-check `doctor`, and **the engine daemon** |
| `compass-worker-host` | 187 | the extension host: framing, sandboxed spawn, 45 of tsapi's 49 methods, and the real runtime |
| `compass-xdg` | 174 | desktop entries, locale, exec, reader, mimeapps, bookmarks — scope gaps listed below |
| `compass-clipboard` | 114 | history store, ingest, migrations, and the history command's own decisions; stored enums pinned to the C++ header |
| `compass-extension-api` | 73 | view tree, derived identity, diff, dispatch, capabilities, controlled inputs |
| `compass-ipc` | 73 | framing, transport, single-instance |
| `compass-search` | 58 | fuzzy, plus an exact port of fzf's coherence rule |
| `compass-portals` | 54 | XDG portals; availability is a three-state outcome, not a boolean |
| `compass-shell` | 46 | GNOME Shell DBus client; tests spawn a real `dbus-daemon` |
| `compass-ui` | 60 | the launcher window and its views, and the root list's sections and selection |
| `compass-crypto` | 24 | AES-GCM and HKDF; cross-decrypted against the C++ probe per-PR |
| `compass-sandbox` | 23 | Landlock, a seccomp denylist, and the launcher that applies them to itself |
| `compass-local-storage` | 36 | the extension key-value store, lossy typing and all, and the calculator history with its time grouping |
| `compass-media` | 13 | MPRIS players, with the timeout the C++ has for a reason |
| `compass-power` | 11 | logind; one C++ bug deliberately not reproduced |
| `compass-db` | 10 | the shared migration runner and the `vicinae` schema |
| `compass-oauth-store` | 9 | the extension token store |
| `compass-sqlcipher-sys` | 8 | SQLCipher and the vendored tokenizer; connection pragmas pinned to the C++ |
| `compass-testkit` | 8 | corpora — **757 desktop entries, 738 harvested from real hosts** |
| `compass-notify` | 7 | desktop notifications over D-Bus |
| `compass-platform` | 6 | the launcher seam (ADR-0013) |
| `compass-platform-linux` | 26 | the launcher, and the uinput virtual keyboard's protocol |
| `compass-wayland` | 33 | activation and keyboard inhibit, and the clipboard offer filter |
| **Total** | **2,712** | what `make check-rust` reports, doctests included, all green under fmt and clippy `-D warnings` |

The per-crate column is measured with `cargo test -p <crate> --all-targets` and sums to 2,071;
the 2,712 is the workspace figure `make check-rust` prints, which additionally covers doctests and
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
| `src/data-control-server` | `compass-wayland` | Phase 5 | ✅ | 🟡 | ✅ | ❌ |
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
| `src/services/font-service` | `compass-core` | Phase 5 | ✅ | 🟡 | ✅ | ❌ |
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
| `src/services/script-command` | `compass-core` | Phase 5 | ✅ | 🟡 | ✅ | ❌ |
| `src/services/selection` | `compass-core` | Phase 3 | ✅ | 🟡 | ✅ | ❌ |
| `src/services/shortcut` | `compass-core` | Phase 5 | ✅ | 🟡 | ✅ | ❌ |
| `src/services/shortcut-inhibit` | `compass-core` | Phase 3 | ✅ | 🟡 | ✅ | ❌ |
| `src/services/snippet` | `compass-core` | Phase 5 | ✅ | 🟡 | ✅ | ❌ |
| `src/services/telemetry` | `compass-core` | Phase 5 | ✅ | 🟡 | ✅ | ❌ |
| `src/services/toast` | `compass-core` | Phase 4 | ✅ | ✅ | ✅ | ❌ |
| `src/services/tray` | `compass-core` | Phase 5 | ✅ | 🟡 | ✅ | ❌ |
| `src/services/tray-host` | `compass-core` | Phase 5 | ✅ | 🟡 | ✅ | ❌ |
| `src/services/update` | `compass-core` | Phase 5 | ✅ | 🟡 | ✅ | ❌ |
| `src/services/url-scheme` | `—` | n/a (Windows) | ✅ | n/a | n/a | ❌ |
| `src/services/wallpaper` | `compass-core` | Phase 5 | ✅ | 🟡 | ✅ | ❌ |
| `src/services/window-manager` | `compass-core` | Phase 3 | ✅ | ❌ | ❌ | ❌ |
| `src/services/window-material` | `compass-core` | Phase 5 | ✅ | 🟡 | ✅ | ❌ |

## Builtins

| C++ source | Rust home | Phase | C++ ✓ | Rust ✓ | parity test ✓ | C++ deleted ✓ |
|---|---|---|:-:|:-:|:-:|:-:|
| `src/builtins/browser` | — | **out of scope** | ✅ | n/a | n/a | never |
| `src/builtins/calculator` | `compass-core` | Phase 5 | ✅ | 🟡 | ✅ | ❌ |
| `src/builtins/clipboard` | `compass-clipboard` | Phase 5 | ✅ | 🟡 | ✅ | ❌ |
| `src/builtins/developer` | `compass-core` | Phase 5 | ✅ | 🟡 | 🟡 | ❌ |
| `src/builtins/file` | `compass-core` | Phase 5 | ✅ | 🟡 | ✅ | ❌ |
| `src/builtins/font` | `compass-core` | Phase 5 | ✅ | 🟡 | 🟡 | ❌ |
| `src/builtins/internal` | `compass-core` | Phase 5 | ✅ | ✅ | ✅ | ❌ |
| `src/builtins/media` | `compass-core` | Phase 5 | ✅ | 🟡 | ✅ | ❌ |
| `src/builtins/power-management` | `compass-core` | Phase 5 | ✅ | 🟡 | 🟡 | ❌ |
| `src/builtins/raycast` | `compass-core` | Phase 5 | ✅ | 🟡 | ✅ | ❌ |
| `src/builtins/root` | `compass-core` | Phase 5 | ✅ | 🟡 | ✅ | ❌ |
| `src/builtins/shortcut` | `compass-core` | Phase 5 | ✅ | 🟡 | 🟡 | ❌ |
| `src/builtins/snippet` | `compass-core` | Phase 5 | ✅ | 🟡 | 🟡 | ❌ |
| `src/builtins/system` | `compass-core` | Phase 5 | ✅ | 🟡 | 🟡 | ❌ |
| `src/builtins/theme` | `compass-core` | Phase 5 | ✅ | 🟡 | 🟡 | ❌ |
| `src/builtins/vicinae` | `compass-core` | Phase 5 | ✅ | 🟡 | ✅ | ❌ |
| `src/builtins/wm` | `compass-core` | Phase 5 | ✅ | 🟡 | ✅ | ❌ |

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
- the sibling modules below (the `DesktopFile` layer itself is now ported as
  `compass_xdg::desktop_file`: `relativeId`, `fromId`'s two-candidate lookup, and the standalone
  filename id, with 24 tests and 16 controls);
- the sibling modules `bookmark`, `env`, `file-uri`, `file`, `mime`, `special`.


#### An unresolved disagreement: two desktop file id schemes

Porting `relativeId` turned up a disagreement between the two engines that nothing was recording.

The XDG Desktop Entry Specification says a desktop file ID is the path below the applications
directory with `/` turned into **`-`**. The C++ turns it into **`.`**. For a file directly in the
directory the two agree; for anything nested they do not — `kde4/konsole.desktop` is
`kde4-konsole.desktop` by the specification and `kde4.konsole.desktop` by the C++.

This matters because the id is the key. `xdg-app-database.cpp` keys every application by
`relativeId`, and that id is what an application's frecency score, alias, and enabled or disabled
state are stored under. `compass_xdg::scan` keys by `desktop_file_id`, which follows the
specification. **So the two engines would not find each other's records for any nested
application** — a user moving from one to the other would silently lose per-app state for anything
installed in a subdirectory, which is where distribution-packaged KDE and GNOME applications often
live.

Both functions now exist, named for what they are, and a test pins the disagreement in both
directions so neither can be changed by accident. Which one the Rust engine should key on is a
decision about migrating stored data — keep the C++'s and inherit its divergence from the
specification, or keep the specification's and migrate existing records — and that belongs to
whoever owns the data, not to whichever function a caller reached for first. It is recorded here
rather than resolved.

The dotted scheme has a second property worth knowing if it is kept: because the `.desktop` suffix
is part of the id, it is indistinguishable from a separator. An id of `kde4.konsole.desktop` could
be a nested `konsole` or a flat file of that exact name, and nothing recovers the difference.

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

**`src/data-control-server` → `compass-wayland::data_control`** — the part that decides which of a
Wayland client's offered MIME types belong in a clipboard entry, and which of them get their bytes
read over a pipe. Ported: the accept test (`text/`, `image/`, `application/`, plus
`x-special/gnome-copied-files` by name) with `application/x-qt-image` checked against the ignore
list **first**, so the hint that an image exists does not survive its own prefix; the flag types,
kept for their presence and never read, which is the only way a password hint survives at all;
the single-image rule, where a later encoding must be **strictly** better ranked to displace an
earlier one, so two encodings the preference list does not name keep the first offered rather than
the last; and the drop of `text/plain` when `text/plain;charset=utf-8` is also there, matched on
that one spelling and not on any charset. The primary selection is much stricter and is ported that
way: a concealed selection is dropped without being read, only plain text is kept (UTF-8 first, and
the loop breaks so both spellings never both appear), and a selection over 1 MiB is dropped rather
than truncated *and not retried* with the other spelling.

One test here is not control-backed and says so in place: the offers come back in lexicographic
order because the C++ collects into a `std::set` and the port returns a `BTreeSet`, so the ordering
is a guarantee of the type rather than behaviour a mutation could change. Still C++-only: the
Wayland plumbing itself — the registry, the seat, the data device and offer objects, the pipe
reads, and the process that carries them.

**`src/builtins/vicinae` → `compass-core::{emoji_grid, bug_report}`** — the largest builtin
directory (2,800 lines across 45 files). Two of its pieces are ported, the two with arithmetic and
rules in them rather than view plumbing.

**The emoji picker.** A skin tone goes after the **first** codepoint, not at the end, because in a
person-joined-to-an-object sequence the tone belongs to the person. Every variation selector in what
follows is then stripped, and the C++ comment gives a reason that is not cosmetic: a tone modifier
already forces the coloured presentation, and leaving the selector in produces a sequence some fonts
refuse to compose, so the glyph breaks apart into its parts. The `Default` tone carries an **empty**
modifier, which is what makes applying it a no-op rather than a special case in the applier. A
glyph's own remembered tone beats the picker's, because someone who set a tone on one glyph meant
that glyph. The copied codepoint is the **first** one only — a sequence reports the codepoint of the
thing it is a sequence *of*, which is the number someone looking it up wants.

The tone menu's two exclusions each do something. Skipping the tone already in force keeps the panel
from offering to do nothing; skipping the picker's default keeps it from offering a second route to
what the "reset to preference" row already does — and that row only appears when the glyph is
actually overriding, because otherwise there is nothing to reset to. Only copying and pasting
register a visit: copying a glyph's *name* or codepoint is looking something up, and counting it
would let a search for a name drift the picker's ordering.

**The bug report.** The template was copied out of the C++ mechanically and its measurements are
pinned. An extension bug goes to a different repository *and* a different path
(`/issues/new/choose`, not `/issues/new`), because that repository offers templates. An empty title
is left out rather than sent empty, which would leave GitHub's own placeholder unused and the field
looking filled in. The two OS descriptions are deliberately different shapes — a dash between the
os-release fields, parentheses around the architecture — so a reader can tell which one they are
looking at.

**The fallback manager.** A command that cannot be a fallback appears in *neither* list: the manager
is not a list of everything with a switch beside it, and showing commands that cannot be turned on
would be showing switches that do nothing. The enabled list is ordered by the stored fallback order
rather than by relevance, because a fallback's position decides which of them answers a query first
— any other order would show a ranking that is not the one in force.

Still C++-only, and it is most of the directory: the store views and detail host, the installed
extensions list, the OAuth token store, the local-storage browser, the menu-bar and tray searches,
the builtin-icon gallery, and the extension registration in `vicinae-extension.cpp`.

**`src/builtins/root` → `compass-core::root_view`** — the root list's own behaviour: the clock in
the title bar, the space-bar alias shortcut, and reaching back through past searches with the up
arrow.

The clock's next tick is `interval - (now % interval)`, which lands on a multiple of the interval
rather than one interval from now — so a clock showing minutes updates *on* the minute instead of
drifting to whenever the window happened to open, and one re-enabled mid-interval falls back into
step with a single short tick. Turning it off clears the title as well as stopping the timer, which
matters as much: leaving the last time on screen would show a clock that had silently stopped.

The space-bar shortcut fires only when what was typed *is* the selected item's alias, compared
lowercased. With a completer open the space goes to the completer instead — but only while every
completion field is still empty, because once something has been typed into one, stealing the space
would make the arguments unwritable. Without a completer the item has to support the shortcut; a
no-view command does not, as there would be nothing to show for it. The completer branch returns
*before* the support check, so a completer overrides the item's own answer either way.

Reaching history with the up arrow needs two things, and the first is a **conflict rather than a
preference**: wrapping navigation makes the up arrow at the top jump to the bottom, so it cannot
also mean "previous search" — where wrapping is on, history is unreachable by design. The second is
that the selection is already on the first row. The first press takes offset 0, so one press reaches
the last thing typed, and the skip past an entry equal to the current text is a **loop**: history
can hold the same query several times in a row, and stopping after one would leave the arrow doing
nothing on the second press.

A control found a gap the suite had: nothing covered an empty query against an item with **no**
alias — the one case that separates an absent alias from an empty one, where treating the two alike
would make every press of space over an empty search box activate whatever was selected. There is a
test for it now.

Still C++-only: the search sources and their models (`root-search-sources.cpp`,
`root-search-model.cpp`, 768 lines between them), the provider search view, and everything the
actions do.

**`src/builtins/clipboard` → `compass-clipboard::history_view`** — the history command's own
decisions. The ledger had this row down for `compass-core`; it landed in `compass-clipboard`
instead, because everything it decides is decided *about* `OfferKind` and `EncryptionType`, which
live there — and `compass-core` does not depend on that crate. The crate column was a plan; the
types decide.

The query controller is **single-flight with one waiting slot**, and the port keeps both halves. At
most one query runs and at most one waits; a third request while one is running collapses into the
same slot rather than queueing, because only the newest matters. And a result that arrives while
something newer is wanted is **dropped without ever reaching the list** — the C++ comment says why,
and it is not about wasted work: each delivery consumes the view's one-shot "select the first row"
flag, so a stale delivery would move the selection out from under whoever was reading. The same
flag is what makes every delivery after the first *incremental*: a pin, a rename, or a new copy
landing while the list is open leaves the selection where it is.

The action panel is gated in three independent ways. An entry whose contents cannot be read — an
encrypted one before the keyring is unlocked — offers a way into the settings and **neither** copy
nor paste, rather than an action that would fail. Where pasting is unsupported only copying appears.
And where both appear, the stored preference decides only their *order*, the first being what the
return key runs; only the exact string `paste` selects pasting, and everything else falls back to
copying, which is the safer of the two to get wrong.

Opening is ported with its asymmetry. A file entry holds a URI list, and open actions appear only
when it holds **exactly one** entry that still exists — a copy of three files has no single thing to
open, and a copy of one that has since been deleted would offer to open nothing. A link needs no
such check, because the payload *is* the target. In both cases the chooser appears whenever the
target is usable and `open` additionally needs a default application, so a file type nothing claims
still gets a chooser.

The filter's stored vocabulary is the enum's and not the interface's — the option reads `Images` and
stores `image` — and keeping them apart is what lets either change without the other. A kind with no
option (`Unknown`, and the enum's count sentinel) answers index 0 rather than an out-of-range index
that would select nothing.

One fixture was too permissive and a control caught it: an `exists` stub that accepted every path
let a mangled path through, so splitting the URI list on the wrong separator — which leaves a stray
carriage return — looked correct. The stub now names the paths it knows.

Still C++-only: the QML views, the detail pane, the drag payload, and the actions' effects.

**`src/builtins/raycast` → `compass-core::raycast_store_view`** — the store's two views. Its API
client was already ported (`compass-core::raycast_store`); this is what the views do with what it
returns.

The list is a **rendezvous, not a sequence**. The page and the compatibility sheet are fetched
independently and may finish in either order, and whichever lands second is what draws the list —
drawing on the page alone would show every row with no badge and then flicker when the sheet
arrived. The two stale-result guards differ correctly: the browsable list is only ever shown for the
empty query, so it checks that the box is *still* empty, while a search compares against the query
it was sent for.

Three distinctions in the compatibility handling are ported because each means something different.
A platform with **no sheet** shows no badge at all, which is not the same as an `Unknown` badge —
one says the question does not apply, the other that it was asked and not answered; the view model
separates them with `-1`. An extension the sheet does not mention gets a *different* muted banner
from one the sheet mentions with an unrecognised status: "may or may not work" against "no data is
available". They look identical and read differently, and only the first can be fixed by someone
adding a row to the sheet. And an unrecognised status is `Unknown` rather than an error, so a status
the sheet gains later degrades instead of breaking.

`formatCount` is ported with its arithmetic intact: both thresholds are **strict**, so exactly 1,000
prints as `1000` and 1,000,000 prints as `1000K`; and the figure is rounded to one decimal by
*ceiling*, so 1,001 downloads reads as `1.1K`. That is generous and it is what ships.

**Ported as-is rather than fixed:** a failed fetch leaves the spinner running. Both handlers report
the failure and return before clearing the loading state. It is visible behaviour, and correcting it
here would make the two implementations disagree while the C++ is still the one shipping, so it is
pinned by a constant and a test instead.

One line was written and then removed because a control could not make it fail: a special case for
printing a whole number without its fraction. Rust's `f64` `Display` already does that, the same way
`QString::arg(float)` does.

Still C++-only: the HTTP calls themselves, the install-from-zip path, and the QML views.

**`src/builtins/internal` → `compass-core::internal_commands`** — a hidden extension holding one
command: a fixed Markdown document rendered to check that every construct the renderer claims to
support actually renders. This row is **green rather than partial**, because there is nothing else
in the directory — the view that displays the document is shared with the store intro and belongs to
that row.

The document is a *test fixture that ships*, and it is treated as one. It was copied out of the C++
raw string literal mechanically rather than retyped, because a fixture whose job is to exercise a
renderer is exactly where a character typed wrong still looks right in review; its byte and line
counts are pinned so a later edit cannot quietly resize it. The tests then enumerate what it must
contain — five heading levels, the five inline styles, both kinds of line break, six labelled code
languages *and* an unlabelled fence, both list kinds, a table with all three column alignments,
inline formatting inside a list item and inside a table cell, both blockquote shapes including the
empty quoted line that makes a second paragraph, all five callout kinds, a plain image and an image
wrapped in a link, a horizontal rule, and the closing line that is the only way to see from the
rendered page that nothing was truncated. Each of those is a separate path through the renderer, so
a missing one is a missing code path rather than a missing sentence, and every one of them has a
control that deletes it from the document.

One thing is kept as it is: the extension's display name and description are the same string in the
C++. It is not meant to be found by searching, so a description distinguishing it from its own name
would be describing it to nobody.

**`src/builtins/file` → `compass-core::file_search`** — when a query is read as a path rather than a
search, which of three result modes the view is in, how a late answer is discarded, and how the
category filter is stored and read back.

The path test is deliberately anchored: `~`, `.` and `..` count only as the *whole* query, and the
prefixed forms need their separator, so `~notes` stays a search rather than becoming a home-relative
path that does not exist and `notes..txt` is not mistaken for a traversal. The direct-path branch
then needs three things at once — the text must look like a path, the path must exist, and it must
not be `/`. That last one matters: the root exists everywhere, and matching it would turn a single
slash into a one-item list instead of a search for names containing a slash. When a category filter
excludes the named file the result is an **empty** direct-path section rather than a fall-through to
the index: the user asked for that file, and answering with a list of other files would be a
different question.

The three stale-result guards are ported as three, because each catches something the others do not:
the task may have been cancelled, the view may have changed mode (a direct path typed while a search
was in flight), and the query may have moved on — the last being what stops out-of-order answers
leaving the list showing a question already finished with. Recent files are guarded differently and
correctly so: they are only ever requested for the empty query, so the test is that the box is
*still* empty rather than that it matches a remembered string.

The filter stores the **untranslated** key, so a filter chosen in one language is still readable in
another, and `restored_filter_index` folds three cases into one: nothing stored, an unknown value,
and `All` all restore nothing — which is what the C++'s `index <= 0` means, `-1` being not-found.

Two lines are noted rather than silently kept or dropped. The C++'s empty-string guard in the path
test is unreachable here (none of the tests below can match an empty string) and is left out rather
than carried over as a line no mutation can reach. The `index >= 0` bound check *is* kept although
the cast to `usize` already covers it, because it says what is meant and survives a future signed
comparison. Rebuilding the index is written and deliberately unregistered in the C++, with a comment
saying the indexer's timed sweeps and deleting its cache directory have the same effect; the port
keeps it unregistered for the same reason, and a test pins that.

Still C++-only: the indexer behind the search, the file preview in the detail pane, the drag payload
and the per-platform preference sets.

**`src/builtins/media` → `compass-core::media_commands`** — which commands exist on which platform,
how a player is chosen from what was typed, what the on-screen display says, and which speaker glyph
goes with a volume. The player commands need MPRIS and are registered only where it exists; the
volume commands go through the audio service and are registered everywhere, so a platform without
MPRIS gets a shorter list rather than commands that fail.

`trackLabel` narrows in a fixed order and the order is only visible in one case: with a title but no
artist, or an artist but no title, either order gives the same answer — it is the player with
*neither* that shows it, where testing the artist first would return the empty title and leave the
row blank. A test covers exactly that case. Choosing a player is two different things: an empty
query takes the player the media service already considers active, and anything else is a search.
Their two failure messages are different sentences on purpose — one is a fact about the system, the
other quotes the query back, which is what says "you misspelled it" rather than "your music
stopped".

The play/pause message is built from the state *before* the toggle, because reading it back after
would race the player's own reply. A skip asks the player whether it can before calling, so the
refusal names the player instead of reporting a bare failure.

The four speaker bands put each boundary in the *lower* band, and silence is its own case rather
than the bottom of the first, so a muted system shows a crossed-out speaker and not a quiet one.
The percentage is rounded half away from zero, the way `qRound` does and unlike Rust's default
`round`-to-even would be if written casually — so a nudge that changed something never reads as if
it changed nothing.

Two things are ported as they are rather than tidied:

- The preset list disagrees with `volumeIcon`. 50% is listed with the *low* speaker while
  `volumeIcon(0.5)` returns the *down* one. The list is written out by hand in the C++ and drifted
  from the function. Changing either changes what someone sees today and neither is more right, so
  both are kept and a test pins the disagreement.
- `volume-down 5` turns the volume **up** by five. Both commands share one `adjustVolume` call and
  neither negates its argument, so the defaults are the only thing carrying the direction. Pinned
  rather than quietly corrected.

Still C++-only: the MPRIS provider, the audio provider, and the Now Playing view.

**`src/builtins/wm` → `compass-core::window_switcher`** — which commands the window-management
extension offers and how a window and a workspace are described in the list. Switching windows is
unconditional; everything else is gated on a capability, because a command certain to fail is worse
than a command that is absent. The C++ also tests `SetSticky` and registers nothing in that branch;
the port keeps the empty test rather than tidying it away, because the capability *is* used — a
window's action panel offers pinning when it is present — so the branch is the only written record
that someone meant a command to go there.

A window's row falls back twice and the two are independent: the subtitle is the application's
display name or the raw `WM_CLASS`, and the icon is the application's or the generic window glyph.
The accessory has three distinct cases, not two: a named workspace shows its name, a numbered one
shows `WS n`, and a window on no workspace shows nothing at all — an absent accessory is not the
same as `WS ` with nothing after it. The rule that decides between the first two is the subtle one:
a compositor with no workspace names reports the **id as the name**, so a name equal to the id is
treated as no name, which is what keeps a bare `3` out of the slot where a name belongs. Searching
keeps the `WM_CLASS` at low weight even when an application was recognised, so someone who knows a
window as `org.gnome.Geary` still finds it when the desktop entry calls it Mail.

A workspace's applications are deduplicated but its windows are counted, so three terminals show one
icon and the subtitle still says three; an unrecognised window is counted and contributes no icon.
The applications are searchable at low weight, which is what lets someone find "the workspace with
the browser on it" without knowing its name. The Windows/other naming split is a compile-time
`#ifdef` in the C++ and an argument here, so both namings are reachable from one build and both are
tested.

**A declared divergence.** The C++ writes the count as Qt's `tr("%n window(s)", "", n)`. Qt applies
plural forms only where a translation supplies them, and there is no English entry for this string —
Russian and Ukrainian have real plural forms and English falls back to the source text. The shipped
English therefore reads `3 window(s)`, with the translator's placeholder left in the interface. The
port writes `1 window` and `3 windows`: what every translated locale already does, and what English
would do if the entry existed. A test pins it.

Still C++-only: the window manager providers themselves and everything the actions do.

**`src/builtins/calculator` → `compass-core::calculator_history`, and the grouping in
`compass-local-storage::calculator`** — the view's own decisions and the half of `CalculatorService`
that is not persistence. The live-calculation gate is ported with both of its rules: three
characters before the search box is also read as a sum (below that almost anything parses as
*something*, and a result flickering in on the way to typing a word is worse than none), and a
leading `=` that gets under the length rule entirely and is stripped before the rest is computed —
including the bare `=`, which the C++ hands to the backend to decline rather than short-circuiting.
The length is counted in characters, so a two-character accented word does not slip through a byte
count. Also ported: the `question = answer` title with its spaces, the conversion/arithmetic icon
split with its `default` arm, and both action panels as sections — pinning alone, the three copies
with **the answer** primary, then the two destructive actions behind a section break, which is the
only thing standing between them and the primary action.

`group_records_by_time` is a **sequential scan, not a classification**, and the port keeps it that
way because the difference is visible. Each group consumes a *prefix* of the rows and stops at the
first that does not match, leaving the rest to the next group; nothing rewinds. Two things follow.
A row older than every boundary reaches `A few years ago` without any group needing a lower bound.
And a row out of order cannot go back to an earlier group — which is safe only because the query
sorts `pinned_at DESC, created_at DESC`, so the `ORDER BY` and this scan are one mechanism and not
two. Every group is produced on every call, including the empty ones, and the view drops those;
that split is the C++'s and is pinned on both sides. `query` is a filter and not a ranking, and an
empty query short-circuits to the full list rather than matching everything — which is what makes
an empty search box show the grouped history rather than a fuzzy-ordered one.

The calendar arithmetic is **not** ported: `group_records_by_time` takes the eight boundary instants
as an argument rather than reading a clock. Computing them belongs to whoever owns the clock, and
keeping them out is what lets the scan be tested without freezing a timezone. The `dividers` vector
the C++ declares at the top of that function is dead — nothing reads it — and is not carried over.
Still C++-only: the backends (unported by design, see the crate docs), the preference dropdown that
selects one, and the refresh-rates command.

**`src/services/script-command` → `compass-core::script_scan`** — the header parser was already
ported (`src/lib/script-command`); this is the layer around it. The scan's rules are ported with
their order intact, and the order is observable: a directory is classified **before** the duplicate
check, so a directory never consumes an id and one named like an already-seen script still
contributes its children; and the duplicate check comes **before** the extension check, so nothing
a rejected `.md` does can shadow a real script of the same id found later in the walk. Also ported:
the `.template` marker matched **anywhere** in a name rather than only as a suffix, the
`depth + 1 < MAX_DEPTH` comparison that lists a directory at depth 4 without opening it, the
"is this text?" test that is nothing more than a NUL byte in the first 8 KiB (so an empty file
passes and reaches the parser), custom directories searched before the packaged ones so a user's
script shadows a stock one, and the case-**sensitive** extension check that lets `README.MD`
through. The command line is ported with its zip: extra values are dropped rather than appended and
a short call passes fewer arguments rather than empty ones, and `percentEncoded` is applied per
argument over bytes, not characters. `packageName` shows an inline script's last line of output —
`No data` until it has run — in the slot a package name would occupy, so declaring one on an inline
script has no effect. The icon chain is emoji, path as written, path beside the script, `https`
URL, then the tinted `code` glyph; `http` is refused because the C++ tests the scheme for `https`
exactly. The metadata store keeps each line base64-encoded for the reason the C++ comment gives —
the output is arbitrary bytes from someone else's script — and a corrupt file leaves an empty store
rather than an error, because a lost output cache is no reason to stop listing scripts. Still
C++-only: the service's own Qt machinery (the filesystem watcher, its 100 ms debounce and the
15-minute refresh), the output tokenizer, the script actions and the executor view host.

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
another nine tests and nine controls.

The manager's state changes are ported too, and this note previously understated that:
`mergeConfigWithMetadata` and `registerVisit` were already done when it was written, and the config
writes behind `setAlias`, `setShortcut`, `setItemEnabled` and `setProviderEnabled` are done now —
seventeen more tests and fourteen controls. A write is a *merge*: setting an alias must not clear a
shortcut set earlier, and the provider and entrypoint entries are created on first write because the
config file holds only what the user changed. Each pairing writes **both** halves, memory and file,
because doing only the first is a change that shows immediately and vanishes on restart.

`setItemEnabled` deliberately writes only the file. The merge applies the provider's setting *after*
the item's, so enabling an item whose provider is off does not make it appear — and only the merge
knows about the provider. There is a test for that interaction end to end rather than for the
absence of a line.

**A declared divergence in `setShortcut`.** The C++ is asymmetric and the asymmetry is a bug: an
empty shortcut *resets* the metadata but still writes `std::string{""}` into the config. On the next
merge that stored empty string is present, so `if (auto shortcut = itemConfig->shortcut)` takes it
and the metadata comes back as `Some("")` rather than `None`. Clearing a shortcut therefore looks as
though it worked until the launcher restarts, and then the item has an empty shortcut instead of
none. This port writes `None`, and two tests pin it — one on the write, one on the round trip
through a merge.

Still C++-only: loading items from the providers themselves, which is the extension registry, the
application database and the rest of the backends rather than logic.

**`src/services/tray-host` → `compass-core::tray_host`** — `TrayItem` and `TrayMenuItem` are
ported: the item key is the bus name *and* the object path, because one application can export
several items on one connection; a menu path of `/` means no menu at all; and the icon falls back
through resolved theme path, raw pixmap, theme name, then `application-x-executable`. Each
attention field falls back **on its own**, so an application that sets an attention icon name but
no attention pixmap still shows its ordinary pixmap rather than nothing. `resolve_icon` keeps
`findIconInThemePath`'s first line — an empty name or an empty theme path returns before any
directory is walked — and `best_icon` returns an SVG immediately and otherwise takes a *strictly*
larger file, so equal-sized candidates take the first and the answer does not depend on directory
read order. `plain_label` strips a single `_` mnemonic and folds `__` to one literal underscore,
iterating by `char` so a label with accented text is not cut mid-codepoint. Still C++-only: the
DBus plumbing itself (the StatusNotifierWatcher registration, the `com.canonical.dbusmenu` layout
walk, and the property-change signals), which belongs to whoever owns the bus connection.

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

### `src/server/src/ui/image` stays ❌ although its wire format and its contrast maths are ported

`compass-core::image_url` is a complete port of `url.cpp` — the `icon://` scheme every icon in the
system is referred to by — and `compass-core::contrast` of `contrast-helper.hpp`, which picks a
colour that can be read against another one. Between them, 53 tests and 43 controls. The row stays
❌, for the same reason
`src/file-indexer` does: the directory is 2,154 lines across fourteen files, and what is ported is
one of them. The renderer, the streaming decoder, the painter and the platform icon loaders are the
rest, and they are the part that needs a UI layer.

The contrast maths splits into a standard and a judgement, and they are tested differently on
purpose. `getRelativeLuminance` and `getContrastRatio` are WCAG 2.x, defined to the digit, so they
are pinned against the standard's own values — black on white is exactly 21:1, and a pure green is
0.7152 where a pure blue is 0.0722. `getTonalContrastColor` is not a standard: it is this project's
answer to "given an album cover, what colour do I write on it", and what is pinned there is the
properties it must hold — it moves away from the background, it keeps the background's hue, and it
gives up after thirty steps rather than looping, because on a mid grey no lightness reaches 4.5 and
a search without the bound would never end.

One thing this port does *not* verify is that its HSL conversion agrees with Qt's to the last bit.
The tests hold the properties rather than exact RGB triples for that reason; a caller needing the
same pixel as the C++ would need a cross-check that does not exist yet.

It was worth porting ahead of them because an `ImageURL` is a *string*, not a widget. That string
crosses every boundary in the system: it is what a root-search row stores, what an extension gets
handed back, what an alert carries. Two details in it are load-bearing for data already written
down, and neither is obvious from reading the code once:

`nameForType` returns the *first* table entry for a type, so `Builtin` prints as `omnicast` rather
than `builtin`, and `Https` prints as `http`. Both spellings parse, so the tables are asymmetric on
purpose and reordering them would silently change every URL a new build writes.

And `resolveThemedLocalPath` inserts `@dark` before the first dot *after the last separator*, not
before the last dot: `a.tar.gz` becomes `a@dark.tar.gz`, and `~/.local/share/logo` becomes
`logo@dark` rather than being confused by the dot in the directory above it.


### The first view-layer work: the root list's sections and selection

Every row still 🟡 is 🟡 for the same reason — the model is ported and tested, and the *backend* is
not. Counted across the notes below, what is left is views (4), providers (3), QML (2), MPRIS and
HTTP. That is the engine rather than more transcription, and it is where the remaining Phase 5
percentage lives.

`compass_ui::root_list` is the first piece of it. The launcher's main list is not one list: it is
favourites, then results, then — when nothing matched — the fallbacks, each under its own heading.
The arrangement and the selection moving *through* it are kept out of the Iced `view` function, so
they can be tested in a container with no display server; only the drawing needs a compositor. 31
tests, 29 controls.

The selection arithmetic is deliberately **flat**, and the sections are invisible to it. A heading
is not a position, so nothing ever lands on one and nothing is skipped crossing a boundary — which
is the bug this shape prevents, and one that is invisible until someone tries a list with more than
one section. Two tests walk the whole list rather than taking one step, because a one-step test
passes against an off-by-one that makes the last row unreachable.

Favourites lead the empty query and vanish once something is typed: a favourite that does not match
is not an answer, and keeping it above the results would push the thing asked for down the page. One
that *does* match appears among the results instead — not hidden, just without its special position.
The search excludes favourites for the empty query precisely so the same row is not drawn twice, and
a test checks for the duplicate rather than trusting the flag.

Fallbacks appear only when nothing matched, which is what makes them fallbacks rather than a section
always on screen. An empty section is never drawn, because a heading with nothing under it is a
heading that lies.

### `src/services/window-manager` stays ❌ although its dispatch layer is ported

`compass-core::window_manager` is a complete port of `window-manager.cpp` — which backend gets
picked, and the focus bookkeeping that lets the launcher act on the window the user was in *before*
they opened it. 39 tests and 32 controls.

The row covers 5,380 lines across thirteen files, and twelve of them are per-compositor providers:
Hyprland, GNOME, KDE, X11, Niri, generic Wayland, Windows virtual desktops, and the event listeners
under each. One file of thirteen is not the row, on the same rule that keeps `src/file-indexer` ❌.
**So this moves the Phase 3 gate by nothing**, and that is the right answer rather than a
disappointing one.

What it is worth is the part that is not compositor-specific and therefore not testable by running
one. The provider order *is* the mechanism: the first candidate that says it can run wins, and the
generic Wayland provider is offered **last** because it is good enough for most standalone
compositors and would otherwise claim Hyprland and Niri too — and then neither would get its own
workspace support. A test pins that ordering, with two activatable candidates rather than one,
because with one the first and the last are the same and the test would pass against either rule.

The focus memory is the other half. The launcher takes keyboard focus when it opens, so "the focused
window" is almost always its own, and every action that means "do this to what I was looking at"
depends on remembering. Three rules carry it, and each is pinned:

- A compositor that reports the *frontmost* window is trusted outright and no memory is kept at all,
  because it knows what is in front even while the launcher holds focus.
- The launcher's own window is never remembered and **does not clear** the memory. That is the whole
  point: opening the launcher must not lose what was underneath it.
- Nothing focused clears the memory only when the launcher does not have focus either — otherwise
  the memory is still the answer.

`isOnActiveWorkspace` answers **yes** at every unknown: no workspaces, no workspace on the window, an
empty workspace id, no active workspace. That is deliberate rather than lax. Callers use it to decide
whether to act on a window at all, so on a compositor reporting partial data, saying no would refuse
every action; saying yes only risks acting on a window the user cannot see.

Two C++ looseness are reproduced rather than tightened. An application's windows are matched by class
*or* by the window title equalling the application's display name case-insensitively — loose enough
that a document window titled after its file will not match, but it is what finds a window that
carries no usable class. And the remembered window is checked by **id** on every refresh, which is
what catches a window the compositor destroyed and replaced rather than moved.

### `src/file-indexer` stays ❌ although part of it is ported

`compass-core::entry_filter` is a complete port of `entry-filter.cpp` — the rules deciding which
directory entries the indexer walks into — `compass-core::query_policy` of
`file-indexer-query-policy.cpp`, which decides what a typed query asks the index,
`compass-core::vocabulary` of `vocabulary.hpp`, which decides what words a file is findable by at
all, and `compass-core::query_ranking` of the scoring half of `file-indexer-query-engine.cpp`, which
decides what order the answers come back in. Between them, 158 tests and 117 controls. The row stays
❌ anyway.

It covers 5,646 lines across fifteen files: the SQLite schema and its writer, the query engine and
its policy, the incremental scanner, the scan dispatcher, the filesystem walker and the watchers.
Four files of those fifteen are not the row — about 995 lines of the 5,646, a fifth — and marking it
🟡 would put a colour on this ledger that means "a model landed without its backend" when what
actually happened is "a fifth of the row landed". The percentage in PLAN.md is only worth anything
if a row's colour means one thing. **So this work moves the Phase 5 figure by nothing, and that is
the right answer rather than a disappointing one.**

The ranking is the fourth leg. A fuzzy score alone would rank an editor's swap file above the file
it is a swap of, and `finalreport.pdf` above `report.pdf`. Every multiplier in the engine exists to
stop one of those, and each is now pinned with a test naming what it prevents:

- The substring bonus applies to the **remaining headroom** — `score + (100 - score) * bonus` —
  rather than multiplying the score. A candidate already near 100 gains almost nothing and one at 40
  gains a lot, so the bonus re-orders the middle of the list without letting a weak match overtake a
  strong one, and cannot push anything past 100.
- A token-start match is worth 1.5 and an inner one 1.05. The second is a tie-break rather than a
  ranking, because a query buried inside a longer word is weak evidence; treating it as strong is
  exactly what would put `finalreport.pdf` above `report.pdf`. The boundary test is
  **alphanumeric**, so `2024report` is one word to the ranker as it is to a reader.
- Editor and compiler leavings are **demoted, not removed**: a swap file is sometimes what you are
  looking for right after a crash, and a search that cannot find it is worse than one that ranks it
  last.
- A correction plan is an **and**. One word scoring zero drops the whole plan, because a correction
  finding files that match two of its three words is not a reading of the query but a different
  query. The surviving words are averaged rather than summed, so a three-word plan is not worth
  three times a one-word plan, and each is normalised against what it scores against *itself*, or a
  six-letter word would always outscore a three-letter one on the same quality of match.
- The four sort keys each exist because the one before it ties, and the third is the interesting
  one: a file comes before a directory, because a directory matching as well as a file inside it is
  usually not what was meant — the file is the thing you open.

One line of the C++ is deliberately not carried over, and a control is why: the guard against a
query longer than the text is unreachable here, since the loop's own bound already covers it. The
*empty*-query guard beside it is kept and is load-bearing — `find("")` succeeds at offset 0, which
would make every text a token-start match.

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

The tokenizer is the third leg of the same argument: a file turns up in a search only if one of its
tokens matches what was typed. Split too coarsely and `AnnualReport2024.pdf` is findable only by its
whole name; too finely and the index fills with fragments matching everything. Its two junk rules
are narrow on purpose — anything over 24 bytes, and anything twelve bytes or longer that is entirely
hexadecimal — because the twelve-byte floor is what stops the hash rule eating `deface`, `facade`
and `decade`, all of which are hex-shaped. Porting it turned up two bugs in the port rather than in
the C++: accumulating a token as `char`s corrupts a UTF-8 filename, and the C++'s length limits are
in bytes rather than characters, which makes them stricter for a non-Latin name.

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

### `compass-core::snippet_expander` — an argument value is not marked as a placeholder

Every substitution the expander makes is pushed with `placeholder = true` except one: an argument's
value goes in through `result.parts.emplace_back(it2->second)`, which takes the struct's default of
`false`. So whatever highlights placeholders in a preview shows a clipboard or a date as
substituted and an argument's value as ordinary text.

Reproduced rather than unified. It is arguably the right answer — the person typed that value, so it
*is* their text — and changing it would alter what an existing preview highlights without anyone
having asked. `substituted_placeholders_are_marked_and_arguments_are_not` pins it either way.

Two more worth knowing, both found by writing the tests rather than by reading the code. A shell
placeholder whose command contains spaces must be quoted: the parser ends an unquoted value at the
first space and then drops the whole placeholder, so `{shell code=rm -rf /tmp/x}` expands to nothing
at all. That is the safe failure — nothing dangerous is half-run — and it now has a test saying so.
And the shell result index advances whether or not a result was available, so a run that returned
fewer outputs than there were placeholders shows each remaining one as its own `$(code)` rather than
shifting every later one onto the wrong command.

### `compass-core::font_service` — two tables extracted, not retyped

The category names and the per-script pangrams were pulled out of the C++ with a script and written
into the Rust source mechanically. That is not laziness: a pangram exists to exercise every letter
of a script, and one retyped with a character wrong still looks right to anyone reviewing the diff
— particularly in Thai, Devanagari or Arabic. Earlier in this session three power-command
descriptions were written from memory and all three were wrong, which is the same failure caught
late rather than avoided.

Two classification rules are worth reading twice. A Nerd Font outranks monospace, because a patched
font is almost always a monospace Latin one and without that order the Monospace section would
contain nothing else; both are still tagged, so filtering by either finds it. And a font covering
several distinctive scripts *plus* a European one is filed under Latin rather than under the first
of them — that is a pan-Unicode font, and burying it under Gujarati would hide a general-purpose
font from everyone.

### `compass-core::window_effects` — two registries whose support checks are in opposite orders

`WaylandShortcutInhibitManager::inhibit` tests whether the window already has an inhibitor *before*
it tests whether the compositor still supports the protocol.
`ExtBackgroundEffectV1Manager::apply` does it the other way round: `if (!isSupported()) return
false;` comes first, so a window that already has a blur is told the apply failed once blur goes
away.

Reading it once, that looks like an inconsistency to tidy. It is the right way round. A Wayland
global can be withdrawn while the process runs, and the two stale states are not comparable: a blur
that lingers is cosmetic, while an inhibitor that lingers means the keyboard is still grabbed. A
manager answering "no" for a grab it is still holding would invite its caller to stop tracking it,
and the person is then locked out of their own desktop shortcuts with nothing to release them.

Both orders are pinned, and pinning them needed the support flag to be *settable* — with support
fixed at construction there is no way to reach the case that distinguishes the two, which is why
the port models `isActive()` as something that changes rather than as a constructor argument.

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
