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

Scaffolding, corpora and CI are in place. `compass-xdg` and `compass-search` have landed as the
first two ports: 153 tests across the workspace, all green.

Neither row is fully green, and neither C++ directory may be deleted yet — see the partial markers
and the divergences below. 🟡 means implemented but not to the full scope of the C++ source.

## Libraries and standalone binaries

| C++ source | Rust home | Phase | C++ ✓ | Rust ✓ | parity test ✓ | C++ deleted ✓ |
|---|---|---|:-:|:-:|:-:|:-:|
| `src/lib/xdgpp` | `compass-xdg` | Phase 1 | ✅ | 🟡 | ✅ | ❌ |
| `src/lib/fuzzy` | `compass-search` | Phase 1 | ✅ | ✅ | 🟡 | ❌ |
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

## Not-yet-ported scope

Tracked here so a 🟡 does not quietly become a ✅.

**`src/lib/xdgpp` → `compass-xdg`** — the desktop-entry, locale, value, reader and exec layers are
ported (47 C++ cases, verbatim inputs). Still C++-only:

- the `xdg-terminal-exec` draft extension (`X-TerminalArg*` typed accessors; the keys are readable
  through `Reader` today);
- the `DesktopFile` layer — `fromId`, `relativeId`, directory search. `from_file` and
  `ParseOptions::{id,path}` exist, but id computation and lookup are a separate pass;
- the sibling modules `bookmark`, `env`, `file-uri`, `file`, `mime`, `special`.

## Declared divergences

Behaviour that intentionally differs from the C++ engine. Each is pinned by a test that fails if
the behaviour changes, so a future fix is loud rather than silent.

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

### `compass-search` — nucleo is not fzf

`nucleo-matcher` uses a different algorithm from the C++ fzf port, so absolute scores are on another
scale and are never asserted. The normalized 0-100 `score`/`quality` values match the C++
expectations closely: every `score == 100`, `score == 50` and `quality == 100` assertion ports
verbatim and passes. What does not:

| # | Divergence | Impact | Pinned by |
|---|---|---|---|
| 1 | **No coherence signal.** C++ computes a `coherent` flag inside the backtracking pass from the DP matrix's boundary bonuses and forces `quality = 0` for incoherent matches. nucleo exposes neither the matrix nor an equivalent, and match indices alone cannot reconstruct it. C++ rejects `"time"` against `"Play this game on Steam"` and `"Start Input Method"`; we accept them at quality 69/73 against a gate of 60. | **Largest semantic gap and the most likely source of user-visible false positives.** The gate still does its main job — `"ny"` against a long description scores 38 and is correctly rejected. | the `diverges_*` coherence tests |
| 2 | **One ordering flip** (upstream issue #946). For `"Spo"`, C++ gives `Spotify > Reload Script Directories > Sysprog`; we give `Spotify > Sysprog > Reload Script Directories`. nucleo prefers a short scatter inside one word starting at position 0; fzf's larger word-boundary bonuses pull the other way. Every other ordering case ports and passes. | Low | `diverges_spo_ordering` |
| 3 | **Narrower diacritic folding.** nucleo folds precomposed accents (é, ñ, ü) but not Latin Extended-A stroked/ogonek letters (Ł, ź, đ, ż), so `"lodz"` does not match `"Łódź Express"`. fzf's table covers them. | Affects Polish, Czech and Croatian app names | `diverges_latin_extended_a_is_not_folded` |
| 4 | **Ties that discriminate nothing.** For `"clip"`, all of `Clipboard History`, `Clear Current Clipboard Data` and `Clear Clipboard History` score identically, because nucleo's score depends only on the matched region, not on haystack length or match position. The C++ ordering test passes there only because `stable_sort` preserves input order — so that case discriminates nothing in *either* implementation. | Real discrimination needs a length or match-position penalty layered on top of nucleo | `diverges_clip_ordering_is_a_three_way_tie` |
| 5 | **Char, not byte, offsets** — a deliberate API change. `"Café Bar"`/`"bar"` reports `5..8` where C++ asserts bytes `6..9`. | None; byte offsets are recoverable | the range tests |

Divergence 1 is the one to resolve before `src/lib/fuzzy` can be deleted. Options: layer a coherence
classifier over nucleo's indices, raise `MIN_QUALITY`, or accept looser matching as a product
decision. **Not yet decided.**
