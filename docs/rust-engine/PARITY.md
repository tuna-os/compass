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
| `vicinae` | 184 | CLI, an 11-check `doctor`, and **the engine daemon** |
| `compass-xdg` | 124 | desktop entries, locale, exec, reader — scope gaps listed below |
| `compass-clipboard` | 76 | history store, ingest, migrations; stored enums pinned to the C++ header |
| `compass-ipc` | 74 | framing, transport, single-instance |
| `compass-core` | 74 | app index, frecency, config |
| `compass-extension-api` | 74 | view tree, derived identity, diff, dispatch, capabilities, controlled inputs |
| `compass-search` | 59 | fuzzy, plus an exact port of fzf's coherence rule |
| `compass-portals` | 55 | XDG portals; availability is a three-state outcome, not a boolean |
| `compass-shell` | 47 | GNOME Shell DBus client; tests spawn a real `dbus-daemon` |
| `compass-crypto` | 24 | AES-GCM and HKDF; cross-decrypted against the C++ probe per-PR |
| `compass-worker-host` | 8 | extension-worker framing, pinned to the TypeScript worker |
| `compass-testkit` | 8 | corpora — **757 desktop entries, 738 harvested from real hosts** |
| `compass-platform` | 6 | the launcher seam (ADR-0013) |
| `compass-wayland` | 2 | |
| `compass-platform-linux` | 2 | |
| `compass-sqlcipher-sys` | 1 + 7 | SQLCipher and the vendored tokenizer; connection pragmas pinned to the C++ |
| **Total** | **849** | what `make check-rust` reports, doctests included, all green under fmt and clippy `-D warnings` |

The per-crate column is measured with `cargo test -p <crate>` and sums to 818;
the 849 is the workspace figure `make check-rust` prints, which additionally
covers doctests and harnesses not attributable to a single package. Both numbers
are given rather than one reconciled figure, because quietly picking whichever
is larger is how a count stops meaning anything.

## Progress

Scaffolding, corpora and CI are in place, and **twenty** crates have landed: **984 tests** across
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
| `src/lib/glyph` | `compass-core` | Phase 5 | ✅ | ❌ | ❌ | ❌ |
| `src/lib/script-command` | `compass-core` | Phase 5 | ✅ | ❌ | ❌ | ❌ |
| `src/lib/vicinae-ipc` | `compass-ipc` | Phase 2 | ✅ | ✅ | 🟡 | ⏳ |
| `src/lib/figura` | `compass-ipc` | Phase 2 | ✅ | n/a | n/a | ⏳ |
| `src/lib/common` | `compass-core` | Phase 2 | ✅ | ❌ | ❌ | ❌ |
| `src/lib/linux-utils` | `compass-platform` | Phase 2 | ✅ | ❌ | ❌ | ❌ |
| `src/lib/soulver` | `—` | n/a (macOS) | ✅ | ❌ | ❌ | ❌ |
| `src/cli` | `crates/vicinae` | Phase 2 | ✅ | 🟡 | 🟡 | ❌ |
| `src/file-indexer` | `compass-platform` | Phase 5 | ✅ | ❌ | ❌ | ❌ |
| `src/data-control-server` | `compass-wayland` | Phase 5 | ✅ | ❌ | ❌ | ❌ |
| `src/snippet` | `compass-core` | Phase 5 | ✅ | ❌ | ❌ | ❌ |
| `src/browser-extension` | — | **out of scope** | ✅ | n/a | n/a | never |

## Services

| C++ source | Rust home | Phase | C++ ✓ | Rust ✓ | parity test ✓ | C++ deleted ✓ |
|---|---|---|:-:|:-:|:-:|:-:|
| `src/services/app-runtime` | `compass-core` | Phase 1 | ✅ | ❌ | ❌ | ❌ |
| `src/services/app-service` | `compass-core` | Phase 1 | ✅ | 🟡 | ✅ | ⏳ |
| `src/services/asset-resolver` | `compass-core` | Phase 1 | ✅ | ❌ | ❌ | ❌ |
| `src/services/audio-control` | `compass-core` | Phase 5 | ✅ | ❌ | ❌ | ❌ |
| `src/services/autostart` | `compass-core` | Phase 5 | ✅ | ❌ | ❌ | ❌ |
| `src/services/browser-extension` | — | **out of scope** | ✅ | n/a | n/a | never |
| `src/services/builtin-icon` | `compass-core` | Phase 1 | ✅ | ❌ | ❌ | ❌ |
| `src/services/calculator-service` | `compass-core` | Phase 5 | ✅ | ❌ | ❌ | ❌ |
| `src/services/clipboard` | `compass-clipboard` | Phase 3 | ✅ | 🟡 | 🟡 | ❌ |
| `src/services/desktop-notification` | `compass-core` | Phase 5 | ✅ | ❌ | ❌ | ❌ |
| `src/services/extension-boilerplate-generator` | `compass-core` | Phase 4 | ✅ | ❌ | ❌ | ❌ |
| `src/services/extension-registry` | `compass-core` | Phase 4 | ✅ | ❌ | ❌ | ❌ |
| `src/services/extension-store` | `compass-core` | Phase 4 | ✅ | ❌ | ❌ | ❌ |
| `src/services/file-chooser` | `compass-core` | Phase 2 | ✅ | ❌ | ❌ | ❌ |
| `src/services/files-service` | `compass-core` | Phase 5 | ✅ | ❌ | ❌ | ❌ |
| `src/services/font-service` | `compass-core` | Phase 5 | ✅ | ❌ | ❌ | ❌ |
| `src/services/global-shortcuts` | `compass-portals` | Phase 1 | ✅ | 🟡 | 🟡 | ❌ |
| `src/services/glyph-service` | `compass-core` | Phase 5 | ✅ | ❌ | ❌ | ❌ |
| `src/services/image-fetcher` | `compass-core` | Phase 5 | ✅ | ❌ | ❌ | ❌ |
| `src/services/input-server` | `compass-core` | Phase 5 | ✅ | ❌ | ❌ | ❌ |
| `src/services/keybinding` | `compass-core` | Phase 5 | ✅ | ❌ | ❌ | ❌ |
| `src/services/local-storage` | `compass-core` | Phase 4 | ✅ | ❌ | ❌ | ❌ |
| `src/services/media-control` | `compass-core` | Phase 5 | ✅ | ❌ | ❌ | ❌ |
| `src/services/menu-bar` | `compass-core` | Phase 5 | ✅ | ❌ | ❌ | ❌ |
| `src/services/navigation` | `compass-core` | Phase 2 | ✅ | ❌ | ❌ | ❌ |
| `src/services/news` | `compass-core` | Phase 5 | ✅ | ❌ | ❌ | ❌ |
| `src/services/oauth` | `compass-core` | Phase 4 | ✅ | ❌ | ❌ | ❌ |
| `src/services/paste` | `compass-core` | Phase 3 | ✅ | ❌ | ❌ | ❌ |
| `src/services/permissions` | `compass-core` | Phase 2 | ✅ | ❌ | ❌ | ❌ |
| `src/services/power-manager` | `compass-core` | Phase 5 | ✅ | ❌ | ❌ | ❌ |
| `src/services/raycast` | `compass-core` | Phase 4 | ✅ | ❌ | ❌ | ❌ |
| `src/services/root-item-manager` | `compass-core` | Phase 2 | ✅ | 🟡 | 🟡 | ⏳ |
| `src/services/script-command` | `compass-core` | Phase 5 | ✅ | ❌ | ❌ | ❌ |
| `src/services/selection` | `compass-core` | Phase 3 | ✅ | ❌ | ❌ | ❌ |
| `src/services/shortcut` | `compass-core` | Phase 5 | ✅ | ❌ | ❌ | ❌ |
| `src/services/shortcut-inhibit` | `compass-core` | Phase 3 | ✅ | ❌ | ❌ | ❌ |
| `src/services/snippet` | `compass-core` | Phase 5 | ✅ | ❌ | ❌ | ❌ |
| `src/services/telemetry` | `compass-core` | Phase 5 | ✅ | ❌ | ❌ | ❌ |
| `src/services/toast` | `compass-core` | Phase 4 | ✅ | ❌ | ❌ | ❌ |
| `src/services/tray` | `compass-core` | Phase 5 | ✅ | ❌ | ❌ | ❌ |
| `src/services/tray-host` | `compass-core` | Phase 5 | ✅ | ❌ | ❌ | ❌ |
| `src/services/update` | `compass-core` | Phase 5 | ✅ | ❌ | ❌ | ❌ |
| `src/services/url-scheme` | `compass-core` | Phase 5 | ✅ | ❌ | ❌ | ❌ |
| `src/services/wallpaper` | `compass-core` | Phase 5 | ✅ | ❌ | ❌ | ❌ |
| `src/services/window-manager` | `compass-core` | Phase 3 | ✅ | ❌ | ❌ | ❌ |
| `src/services/window-material` | `compass-core` | Phase 5 | ✅ | ❌ | ❌ | ❌ |

## Builtins

| C++ source | Rust home | Phase | C++ ✓ | Rust ✓ | parity test ✓ | C++ deleted ✓ |
|---|---|---|:-:|:-:|:-:|:-:|
| `src/builtins/browser` | — | **out of scope** | ✅ | n/a | n/a | never |
| `src/builtins/calculator` | `compass-core` | Phase 5 | ✅ | ❌ | ❌ | ❌ |
| `src/builtins/clipboard` | `compass-core` | Phase 5 | ✅ | ❌ | ❌ | ❌ |
| `src/builtins/developer` | `compass-core` | Phase 5 | ✅ | ❌ | ❌ | ❌ |
| `src/builtins/file` | `compass-core` | Phase 5 | ✅ | ❌ | ❌ | ❌ |
| `src/builtins/font` | `compass-core` | Phase 5 | ✅ | ❌ | ❌ | ❌ |
| `src/builtins/internal` | `compass-core` | Phase 5 | ✅ | ❌ | ❌ | ❌ |
| `src/builtins/media` | `compass-core` | Phase 5 | ✅ | ❌ | ❌ | ❌ |
| `src/builtins/power-management` | `compass-core` | Phase 5 | ✅ | ❌ | ❌ | ❌ |
| `src/builtins/raycast` | `compass-core` | Phase 5 | ✅ | ❌ | ❌ | ❌ |
| `src/builtins/root` | `compass-core` | Phase 5 | ✅ | ❌ | ❌ | ❌ |
| `src/builtins/shortcut` | `compass-core` | Phase 5 | ✅ | ❌ | ❌ | ❌ |
| `src/builtins/snippet` | `compass-core` | Phase 5 | ✅ | ❌ | ❌ | ❌ |
| `src/builtins/system` | `compass-core` | Phase 5 | ✅ | ❌ | ❌ | ❌ |
| `src/builtins/theme` | `compass-core` | Phase 5 | ✅ | ❌ | ❌ | ❌ |
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
| `src/server/src/ui/alert` | 279 | `compass-ui` | Phase 5 | ✅ | ❌ | ❌ | ❌ |

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
