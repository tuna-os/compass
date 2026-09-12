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
- **C++ deleted ✓** — the C++ source is gone. Only legal once the three boxes to its left are ticked.

A row may go green with a **declared divergence** instead of exact parity: record it in the
Divergences section below with a rationale. Divergences are declared, never discovered.

Update this file in the same PR that changes a box. It is meant to be read in standup.

## Progress

Nothing is green yet. Scaffolding, corpora and CI are in place; `compass-xdg` and `compass-search`
are the first two ports in flight.

## Libraries and standalone binaries

| C++ source | Rust home | Phase | C++ ✓ | Rust ✓ | parity test ✓ | C++ deleted ✓ |
|---|---|---|:-:|:-:|:-:|:-:|
| `src/lib/xdgpp` | `compass-xdg` | Phase 1 | ✅ | ❌ | ❌ | ❌ |
| `src/lib/fuzzy` | `compass-search` | Phase 1 | ✅ | ❌ | ❌ | ❌ |
| `src/lib/crypto` | `compass-core` | Phase 3 | ✅ | ❌ | ❌ | ❌ |
| `src/lib/glyph` | `compass-core` | Phase 5 | ✅ | ❌ | ❌ | ❌ |
| `src/lib/script-command` | `compass-core` | Phase 5 | ✅ | ❌ | ❌ | ❌ |
| `src/lib/vicinae-ipc` | `compass-ipc` | Phase 2 | ✅ | ❌ | ❌ | ❌ |
| `src/lib/figura` | `compass-ipc` | Phase 2 | ✅ | ❌ | ❌ | ❌ |
| `src/lib/common` | `compass-core` | Phase 2 | ✅ | ❌ | ❌ | ❌ |
| `src/lib/linux-utils` | `compass-platform` | Phase 2 | ✅ | ❌ | ❌ | ❌ |
| `src/lib/soulver` | `—` | n/a (macOS) | ✅ | ❌ | ❌ | ❌ |
| `src/cli` | `crates/vicinae` | Phase 2 | ✅ | ❌ | ❌ | ❌ |
| `src/file-indexer` | `compass-platform` | Phase 5 | ✅ | ❌ | ❌ | ❌ |
| `src/data-control-server` | `compass-wayland` | Phase 5 | ✅ | ❌ | ❌ | ❌ |
| `src/snippet` | `compass-core` | Phase 5 | ✅ | ❌ | ❌ | ❌ |
| `src/browser-extension` | `compass-core` | Phase 5 | ✅ | ❌ | ❌ | ❌ |

## Services

| C++ source | Rust home | Phase | C++ ✓ | Rust ✓ | parity test ✓ | C++ deleted ✓ |
|---|---|---|:-:|:-:|:-:|:-:|
| `src/services/app-runtime` | `compass-core` | Phase 1 | ✅ | ❌ | ❌ | ❌ |
| `src/services/app-service` | `compass-core` | Phase 1 | ✅ | ❌ | ❌ | ❌ |
| `src/services/asset-resolver` | `compass-core` | Phase 1 | ✅ | ❌ | ❌ | ❌ |
| `src/services/audio-control` | `compass-core` | Phase 5 | ✅ | ❌ | ❌ | ❌ |
| `src/services/autostart` | `compass-core` | Phase 5 | ✅ | ❌ | ❌ | ❌ |
| `src/services/browser-extension` | `compass-core` | Phase 5 | ✅ | ❌ | ❌ | ❌ |
| `src/services/builtin-icon` | `compass-core` | Phase 1 | ✅ | ❌ | ❌ | ❌ |
| `src/services/calculator-service` | `compass-core` | Phase 5 | ✅ | ❌ | ❌ | ❌ |
| `src/services/clipboard` | `compass-core` | Phase 3 | ✅ | ❌ | ❌ | ❌ |
| `src/services/desktop-notification` | `compass-core` | Phase 5 | ✅ | ❌ | ❌ | ❌ |
| `src/services/extension-boilerplate-generator` | `compass-core` | Phase 4 | ✅ | ❌ | ❌ | ❌ |
| `src/services/extension-registry` | `compass-core` | Phase 4 | ✅ | ❌ | ❌ | ❌ |
| `src/services/extension-store` | `compass-core` | Phase 4 | ✅ | ❌ | ❌ | ❌ |
| `src/services/file-chooser` | `compass-core` | Phase 2 | ✅ | ❌ | ❌ | ❌ |
| `src/services/files-service` | `compass-core` | Phase 5 | ✅ | ❌ | ❌ | ❌ |
| `src/services/font-service` | `compass-core` | Phase 5 | ✅ | ❌ | ❌ | ❌ |
| `src/services/global-shortcuts` | `compass-core` | Phase 1 | ✅ | ❌ | ❌ | ❌ |
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
| `src/services/root-item-manager` | `compass-core` | Phase 2 | ✅ | ❌ | ❌ | ❌ |
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
| `src/builtins/browser` | `compass-core` | Phase 5 | ✅ | ❌ | ❌ | ❌ |
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

## Declared divergences

None yet. Format:

| Row | Divergence | Why it is acceptable | Decided by |
|---|---|---|---|
