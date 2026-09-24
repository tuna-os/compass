# Crate audit: hand-written code a maintained crate could replace

**Date:** 2026-09-24. **Rule** (AGENTS.md): lean toward off-the-shelf crates over hand-rolled Rust.
Handle a small gap around a crate rather than reimplementing; hand-roll only when every candidate
would regress documented behaviour, and record behaviour differences in `PARITY.md`.

The workspace (~97k lines) already leans on crates where they fit: `nucleo-matcher`, `fend-core`,
`ignore`, `mime_guess`, `aes-gcm`/`hkdf`, `oo7`, `ashpd`, `landlock`/`seccompiler`, `roxmltree`,
`strsim`/`fst`, and iced's Markdown and SVG support. Most large modules (`compass-ui/app.rs`, the
indexer, the worker host) are project logic no crate covers. What a crate *can* replace is below,
with the decision taken and why.

| Code | Lines | Crate | Decision |
|---|---:|---|---|
| `compass-sqlcipher-sys` FFI wrapper, plus ~300 hand-bound `bind_*`/`column_*`/`step` sites in clipboard, db, local-storage, oauth-store and vicinae | ~800 + sites | `rusqlite` 0.40 with `bundled-sqlcipher` | **Replaced.** PLAN §12.0 item 2(b) had deferred this because the bundled SQLCipher was 4.6.1 against our vendored 4.16.0; `libsqlite3-sys` 0.38.2 bundles **SQLCipher 4.14.0** (SQLite 3.51.3), same SQLCipher 4 defaults. `compass_sqlcipher_sys::open` now keys, registers the tokenizer (the crate's only `unsafe`, through `Connection::handle`, compiled against `libsqlite3-sys`'s headers) and applies the pinned pragmas, and returns a `rusqlite::Connection`; the FFI, `Database`, `Statement` and `Transaction` are deleted and every call site uses `named_params!`, `query_row`, `query_map` and `prepare_cached`. On Linux it links the system `libcrypto`, as before. |
| Percent-encoding, five copies (two byte-identical) | ~140 | `percent-encoding` 2.3 | **Replaced.** Shared sets in `compass_core::uri`; the lenient image-URL decoder and the strict bookmark/file-chooser decoders keep their behaviour. |
| `compass-ipc` frame codec | ~90 of logic | `tokio-util` `LengthDelimitedCodec` | **Replaced.** The framing is the crate's; the codec keeps postcard on either side and reads the prefix at a frame boundary so an oversized frame is still reported with its length. |
| `compass-notify` (whole crate, 185 lines + tests) | ~25 of logic | `notify-rust` 4.18 (same `zbus` 5.19, no duplicate) | **Replaced; the crate is deleted.** Its tests checked the `Notify` call's argument layout, which is now the crate's job, and it brings macOS and Windows notifications for later phases. |
| `compass-power`, `compass-media` raw `call_method` | ~950 total | `#[zbus::proxy]` definitions, zbus's own `fdo` proxies; `mpris` 2.x is sync over libdbus | **Replaced the calls, kept the parsing:** logind, GNOME SessionManager, Plasma Shutdown and the MPRIS player are typed proxies, `ListNames`/`GetAll` are `zbus::fdo`; the MPRIS metadata corner cases, the call timeouts and the logind capability fix stay. |
| `compass-xdg` desktop-entry reader, entry, locale | ~1280 | `freedesktop-desktop-entry` 0.8 (MPL-2.0) | **Keep** (PLAN §12.0 item 5). Its `Exec` expansion splits on whitespace with no quoting, which would regress PARITY `compass-xdg` #3, and its directory discovery misses the Flatpak host roots (#95). |
| `compass-xdg/icon.rs` | ~420 | `freedesktop-icons` 0.4 | **Keep.** No way to add the Flatpak host roots without mutating the process environment. |
| `compass-xdg` mimeapps, MIME subclasses | ~790 | `xdg-mime` 0.4 | **Keep.** Nothing mature covers the desktop-prefixed list order. |
| `compass-search/translit.rs` | 218 | `deunicode`, `any_ascii` | **Keep the table** (deliberate under-expansion keeps subsequence matching); **`deunicode` adopted as the fold fallback** for Ł/đ/ħ (closes PARITY `compass-search` #3). |
| `compass-core/contrast.rs` | 268 | `palette` 0.7 | **Keep**; about 80 lines are replaceable, the contrast search and Qt-HSL rounding are not. |
| `slug.rs` | 89 | `slug` 0.1 | **Replaced.** Same results on every pinned case, and it transliterates non-Latin titles where the C++ emptied them (PARITY `compass-core::slug`). |
| `semver.rs` | 130 | `semver` | **Keep.** It is not semver: it takes any number of dotted integers (`1.2`, `2026.09.24`), which the crate refuses; the tag format decides. |
| `compass-script` (Rhai tier, new) | ~2400 | `rhai`, `notify`, `toml`, `time`, `uuid`, `getrandom`, `percent-encoding` | **Crates throughout.** The script language, file watching, manifest parsing, date formatting, UUIDs and URL encoding are all crates; what is hand-written is the view mapping and the sandbox policy. `toml` is the 1.x line, which shares its parser with what the workspace already locks. |
| interval parsers, `script_output.rs` | ~300 | `humantime`, `vte` | **Keep.** `parse_interval` is 25 lines for Raycast's one-unit manifest syntax (`30s`, `5m`); `humantime` would also accept `1h30m` and `5 minutes`, which Raycast rejects, and change the error text. The tokenizer finds links and SGR codes in one resumable pass; `vte` would replace only the escape half and handles malformed sequences differently from the C++ it ports. |

Not replaceable, checked: crypto (already crates), keyring (`oo7`), XDG base dirs (`dirs` plus
Flatpak-specific roots), the uinput keyboard (a protocol model), clipboard filtering, file walking
(`ignore`), MIME detection (`mime_guess`), the calculator (`fend-core`). There is no HTTP client in
the tree yet; the store work will add one (`ureq` or `reqwest`), and remote icons will use it.
