# ADR-0021: The C++ engine leaves the repository; upstream releases are the reference

**Status:** Accepted · **Date:** 2026-09-25 · Supersedes: ADR-0013 decision 3 (macOS and Windows keep
the C++ build until Phases 9–10) · Relates to: ADR-0007, ADR-0017, ADR-0018, `PARITY.md`,
`BENCHMARKS.md`, `HEAD-TO-HEAD.md`

## Context

The parity ledger reads 152 of 152 (`scripts/ci/parity-score.py`). The in-tree C++/Qt engine was
kept for two reasons: as the thing the Rust port was measured against, and as the build macOS,
Windows and screen-reader users ran. The benchmarks already measure the unmodified upstream
Vicinae v0.29.0 AppImage that `scripts/bench` downloads and SHA-checks, not this fork's C++, and the
owner decided: "Delete the C++ tree; we can use the Vicinae releases for benchmarks now."

## Decision

**Removed** (1,421 files, about 20 MiB):

- `src/server`, `src/cli`, `src/lib`, `src/data-control-server`, `src/file-indexer`, `src/snippet`,
  the C++ copies of `src/wayland-protocols` (the Rust crate carries its own) and
  `src/browser-extension` (its native host spoke to the C++ daemon; ADR-0008 left browser control
  out of the port, so nothing connects to the Rust engine).
- The build: `CMakeLists.txt`, `CMakePresets.json`, `cmake/`, `src/typescript/CMakeLists.txt` and
  `src/typescript/cmake/`, the clang-format, clang-tidy, clangd and qmlformat configs, `vendor/` except
  `fuzzy-trigram`, `nix/vicinae.nix` and `default.nix`, the Makefile's CMake targets, and the scripts
  only those targets and workflows called (`scripts/runners/`, the macOS bundle/DMG scripts, the
  Windows PowerShell scripts, `mkappimage.sh`, `qrc-builder.js`).
- CI: `build-linux`, `build-macos`, `build-windows`, `build-appimage`, `build-appimage-image`,
  `macos-dmg` and `cpp-on-target`; the C++ jobs of `release.yml` and the clang-format step of
  `format.yaml`.
- The C++ stub for `HostCommand/run`, which went with `src/server`.

**Moved**, because the Rust build, its tests or its packaging read them:

| From | To |
|---|---|
| `src/lib/glyph/src/glyph.cpp`, `include/glyph/glyph.hpp`, `scripts/` | `crates/compass-core/glyph/` (data `build.rs` parses; never compiled) |
| `src/server/icons/*.svg` | `extra/builtin-icons/` (`build.rs` now takes the names from `@vicinae/api`'s `Icon` enum) |
| `src/server/database/vicinae/migrations/` | `crates/compass-db/migrations/vicinae/` |
| `src/server/database/clipboard/migrations/` | `crates/compass-clipboard/migrations/` |
| `src/lib/script-command/tests/` | `crates/compass-core/tests/fixtures/script-command/` |
| `src/server/translations/*.ts` | `extra/translations/qt/` (input to `scripts/ts-to-ftl.py`; not yet converted) |
| `src/lib/fuzzy/include`, `src/lib/crypto` (Linux backend) | `scripts/bench/upstream/`, byte-identical to upstream v0.29.0 |
| `src/lib/{fuzzy,crypto}/probe/main.cpp` | `scripts/bench/probes/` |

**Replaced:** `figura`, the IDL compiler, is ported to Rust as `crates/compass-figura`, TypeScript back
end only. Its output is byte-identical to the C++ compiler's for every `.fig` file on both sides.
The extension runtime's bindings under `src/typescript/*/src/proto` are now committed, `make figen`
regenerates them, and a test fails when they are stale. Building the runtime needs npm and nothing
else, so the Flatpak, Nix and Arch builds and CI lose their C++23 compiler and CMake.

**Upstream releases are the benchmark and differential reference.** `scripts/bench` already measured
the pinned v0.29.0 AppImage. The scorer and crypto differentials in `rust.yaml` now build upstream's
own sources at that commit from `scripts/bench/upstream/` instead of this fork's copies. Tests that
read constants out of the in-tree C++ either pin upstream's values (`compass-crypto`'s key labels
and keyring names; the migration list is checked against the migration directory) or were removed
where the Rust tests already pin the value.

## Consequences

- There is no in-tree build for macOS, Windows or screen-reader users (ADR-0013, ADR-0018). They run
  upstream Vicinae releases until the Rust engine covers them.
- The Suite 0 root-ranking differential had no C++ engine to run against, and unmodified upstream
  has no `rootQuery`; its CI job keeps the self-parity run that proves the harness. An upstream
  ranking comparison needs the instrumentation patch in `scripts/bench`, as `HEAD-TO-HEAD.md` says.
- `PARITY.md`'s C++ source paths are history. `parity-score.py` reads only the ledger.
- The on-disk formats stay upstream's: the migrations, the enum values in the clipboard store, the
  key labels and keyring entry. Renaming the product must not rename them.
