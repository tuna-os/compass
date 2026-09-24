# Crate audit: hand-written code a maintained crate could replace

**Date:** 2026-09-24. **Rule** (AGENTS.md): before porting a C++ feature, look for an existing crate;
port only what no maintained crate covers, and record behaviour differences in `PARITY.md`.

The workspace (~97k lines) already leans on crates where they fit: `nucleo-matcher`, `fend-core`,
`ignore`, `mime_guess`, `aes-gcm`/`hkdf`, `oo7`, `ashpd`, `landlock`/`seccompiler`, `roxmltree`,
`strsim`/`fst`, and iced's Markdown and SVG support. Most large modules (`compass-ui/app.rs`, the
indexer, the worker host) are project logic no crate covers. What a crate *can* replace is below,
with the decision taken and why.

| Code | Lines | Crate | Decision |
|---|---:|---|---|
| `compass-sqlcipher-sys` FFI wrapper, plus ~300 hand-bound `bind_*`/`column_*`/`step` sites in clipboard, db, local-storage, oauth-store and vicinae | ~800 + sites | `rusqlite` 0.40 with `bundled-sqlcipher` | **Replace, in its own PR.** PLAN §12.0 item 2(b) deferred this because the bundled SQLCipher was 4.6.1 against our vendored 4.16.0. Checked 2026-09-24: `libsqlite3-sys` 0.38.2 bundles **SQLCipher 4.14.0** (SQLite 3.51.3). The `fuzzy_trigram` tokenizer still needs `unsafe` registration through the raw handle, and the pragma parity test and a cross-open of a C++-written database gate the change. |
| Percent-encoding, five copies (two byte-identical) | ~140 | `percent-encoding` 2.3 | **Replaced.** Shared sets in `compass_core::uri`; the lenient image-URL decoder and the strict bookmark/file-chooser decoders keep their behaviour. |
| `compass-ipc` frame codec | ~90 of logic | `tokio-util` `LengthDelimitedCodec` | **Kept.** Its error does not carry the frame length, which the oversize diagnostics and tests assert; keeping it means peeking at the prefix ourselves, which is most of what would be deleted. |
| `compass-notify` | ~25 of logic | `notify-rust` 4.18 (same `zbus` 5.19, no duplicate) | **Deferred to the macOS/Windows phases**, where it pays for itself. On Linux it is one D-Bus call either way, and the crate opens its own connection, which the spawned-bus tests cannot inject. |
| `compass-power`, `compass-media` raw `call_method` | ~950 total | `#[zbus::proxy]` definitions or `zbus_systemd::login1` | **Replace the calls, keep the parsing:** typed proxies as `compass-shell` already uses; the MPRIS metadata corner cases and the logind capability fix stay. |
| `compass-xdg` desktop-entry reader, entry, locale | ~1280 | `freedesktop-desktop-entry` 0.8 (MPL-2.0) | **Keep** (PLAN §12.0 item 5). Its `Exec` expansion splits on whitespace with no quoting, which would regress PARITY `compass-xdg` #3, and its directory discovery misses the Flatpak host roots (#95). |
| `compass-xdg/icon.rs` | ~420 | `freedesktop-icons` 0.4 | **Keep.** No way to add the Flatpak host roots without mutating the process environment. |
| `compass-xdg` mimeapps, MIME subclasses | ~790 | `xdg-mime` 0.4 | **Keep.** Nothing mature covers the desktop-prefixed list order. |
| `compass-search/translit.rs` | 218 | `deunicode`, `any_ascii` | **Keep the table** (deliberate under-expansion keeps subsequence matching); `deunicode` is a candidate *fallback* to fold Ł/ź/đ (PARITY `compass-search` #3). |
| `compass-core/contrast.rs` | 268 | `palette` 0.7 | **Keep**; about 80 lines are replaceable, the contrast search and Qt-HSL rounding are not. |
| `semver.rs`, `slug.rs`, interval parsers, `script_output.rs` | ~500 | `semver`, `slug`, `humantime`, `vte` | **Keep.** Each is small and pinned by a PARITY note the crate would break (e.g. `semver` accepts `-rc`). |

Not replaceable, checked: crypto (already crates), keyring (`oo7`), XDG base dirs (`dirs` plus
Flatpak-specific roots), the uinput keyboard (a protocol model), clipboard filtering, file walking
(`ignore`), MIME detection (`mime_guess`), the calculator (`fend-core`). There is no HTTP client in
the tree yet; the store work will add one (`ureq` or `reqwest`), and remote icons will use it.
