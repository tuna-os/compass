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
| `qt_date.rs` (a snippet's `{date format=…}`) | ~150 | `jiff`, `time`, `chrono` strftime | **Keep the formatter; `jiff` supplies the clock.** Stored snippets hold Qt format strings (`yyyy-MM-dd hh:mm`); every crate has its own syntax, so a translation would need the same tokenizer. `jiff` is used for the local time and zone, which `time` cannot give a multi-threaded process. |
| Browse Fonts' font enumeration (`vicinae/fonts.rs`) | ~60 | `fontdb` 0.23, `ttf-parser` 0.25 (both already in the tree through iced's text stack); `fontconfig` bindings | **Crates used.** `fontdb` lists the families the renderer will find, and `ttf-parser` answers the cmap lookups that decide each family's scripts. Binding libfontconfig for its language sets would add a C dependency the port otherwise avoids and would not exist on macOS or Windows; the difference is PARITY "Browse Fonts" #1. |
| User theme files (`compass-core::theme_file`) | ~350 | `toml` 1 (already in the tree for Rhai manifests); `palette` for `QColor::lighter`/`darker` | **`toml` used; the colour steps hand-written.** Parsing is the crate's. `palette` would add a dependency for two HSV scalings and a mix, about 40 lines, whose Qt-specific overflow rule (value past 1.0 is taken from the saturation) it does not have. |
| interval parsers, `script_output.rs` | ~300 | `humantime`, `vte` | **Keep.** `parse_interval` is 25 lines for Raycast's one-unit manifest syntax (`30s`, `5m`); `humantime` would also accept `1h30m` and `5 minutes`, which Raycast rejects, and change the error text. The tokenizer finds links and SGR codes in one resumable pass; `vte` would replace only the escape half and handles malformed sequences differently from the C++ it ports. |

Not replaceable, checked: crypto (already crates), keyring (`oo7`), XDG base dirs (`dirs` plus
Flatpak-specific roots), the uinput keyboard (a protocol model), clipboard filtering, file walking
(`ignore`), MIME detection (`mime_guess`), the calculator (`fend-core`).

The HTTP client is `ureq` 3 with native-tls (remote images in extension views): blocking is all a
fetch on a worker thread needs, and native-tls verifies against the system's roots through the
OpenSSL already linked for SQLCipher. URL parsing (the OAuth redirect) is `url`, already in the tree.
`oauth2` was considered and not used: Raycast's PKCE client builds the request and exchanges the
code in the extension, so the host never speaks OAuth itself.

## Phase 5 Track B: the wlroots family (2026-09-24)

| Need | Crate | Decision |
|---|---|---|
| Launcher as a `wlr-layer-shell` surface | `iced_layershell` 0.19.1 (built against `iced_core`/`iced_runtime` **0.14**, so it runs our `LauncherApp` unchanged) | **Used**, in `compass-ui` under a `cfg(target_os = "linux")` target section, default features off (its only default, `mundy` theme detection, duplicates `crate::appearance`). Two costs, both recorded: its `layershellev` needs **`libxkbcommon` headers at build time** (`pkg-config xkbcommon`; the Flatpak SDK has them, Ubuntu CI installs `libxkbcommon-dev`); and its unconditional `iced_exdevtools` 0.19.1 fails to compile against `winit-core` **0.31.0-beta.3** (a new `NativeKeyCode` variant), which `^0.31.0-beta.2` admits — `Cargo.lock` pins `winit-core`/`winit-common` to `0.31.0-beta.2`. A plain `cargo update` would re-break it; drop the pin once `iced_exdevtools` matches exhaustively. |
| Protocol bindings: foreign-toplevel, data-control (ext + wlr) | `wayland-protocols` 0.32 (`staging`), `wayland-protocols-wlr` 0.3 | **Used.** `compass-wayland` moved from `wayland-client` 0.30 to 0.31, the version winit, `iced_layershell` and `wl-clipboard-rs` already link, so the tree carries one Wayland stack for these rather than two. |
| Window list / activate / close | none maintained: crates.io has taskbar applications and screenshot tools that each embed their own client, no library | **Hand-rolled on the protocol crates** (`compass_wayland::toplevel`, ~450 lines incl. tests): a registry bind, one `Dispatch` per interface, a snapshot behind a mutex. |
| Set / read / clear the selection without focus | `wl-clipboard-rs` 0.9 (the library under `wl-copy`/`wl-paste`; ext **and** wlr data-control, chosen at runtime) | **Used** for set (`copy_multi`, served from its own thread), read and clear. |
| *Watch* the selection (clipboard history) | `wl-clipboard-rs` cannot: each call is one connection, one operation. `wayland-clipboard-listener` 0.6 can, but picks ext **or** wlr data-control at **compile time** (Sway before 1.11 has only wlr, newer compositors increasingly only ext), panics on a dispatch error inside its `Iterator`, reads one MIME type per selection, and has ~700 recent downloads | **Hand-rolled the watcher only** (`compass_wayland::clipboard::watch`, ~200 lines): one bound device over whichever manager is advertised, each selection filtered by the existing port of the C++ offer filter (`compass_wayland::data_control`) and read with a per-type timeout. Everything that is not watching stays on `wl-clipboard-rs`. |
| `xx-hotkey-v1` bindings | no crate carries this experimental protocol (it is upstream vicinae's own, #1936) | **Generated, not written**: `wayland-scanner` 0.31 — the generator `wayland-protocols` itself uses — over the checked-in XML, in the new `compass-wayland-protocols` crate. It is a separate crate only because generated interface tables contain `unsafe` and the workspace forbids it; that crate uses `deny` and allows it on the generated module alone. |

### Compositor IPC: Hyprland and niri (2026-09-25)

| Need | Crate | Decision |
|---|---|---|
| niri's socket: `Windows`, `Workspaces`, `FocusedWindow`, the focus/close/workspace actions | `niri-ipc` 26.4 (niri's own crate, GPL-3.0-or-later; serde and serde_json only, default features off) | **Used** for every type: the requests are its `Request`/`Action`, the replies its `Reply`, `Window` and `Workspace`. The one gap is taken around it: `niri_ipc::socket::Socket` sets no timeout, so `compass_platform_linux::compositor::niri` opens the `UnixStream` itself, writes the crate's serialisation and reads one line under a 2 s timeout. Its structs require the fields niri 25.08+ sends (`layout`, `focus_timestamp`); an older niri answers in a shape it cannot read, which is reported and treated as no windows (PARITY "wlroots" #3). |
| Hyprland's request socket: `clients`, `workspaces`, `activeworkspace`, `activewindow`, `dispatch` | `hyprland` 0.4.0-beta.3 (hyprland-rs) | **Hand-rolled** (`compass_platform_linux::compositor::hyprland`, ~300 lines incl. docs). The wire is "connect, write one command, read to EOF", so what a crate would bring is the data types and dispatchers, and both regress the C++: its `Client` is strict and narrow — `at`/`size` as `i16`, `focusHistoryID` as `i8`, a required `fullscreen` enum and `swallowing` address — where the C++ reads eight fields leniently, so a Hyprland release that adds or retypes a field fails the whole list; and its dispatchers speak the classic `dispatch focuswindow address:…` while the C++ dispatches Hyprland's Lua expressions (`hl.dsp.focus({ window = … })`). It is also a beta, pulling `tokio`, `async-stream`, `derive_more` and a proc-macro crate. The replies here are `serde` structs with every field optional and unknown ones ignored. |

## Phase 5 Track A: the extension stores (2026-09-24)

| Need | Crate | Decision |
|---|---|---|
| Fetch the listings, READMEs and bundles | `ureq` 3.4 (already in the tree for remote images) | **Reused**, blocking on tokio's blocking pool with the same native-tls agent. Its workspace feature moved from `native-tls-no-default` to `native-tls`: ureq 3.4 tests `cfg!(feature = "native-tls")` before using the provider, so the old spelling panicked on every `https` request — remote images in extension views included, which no test had fetched over TLS. The cost is `webpki-root-certs`, compiled in and unused (the agents ask for the platform's roots). |
| Unpack a store bundle | `zip` 8.6, `deflate-flate2-zlib-rs` only (flate2 and zlib-rs were already in the tree) | **Used.** `enclosed_name` for the path check, `is_symlink` for links, the reader's CRC-32 check on every entry; the size caps, the refusal of absolute names that `enclosed_name` would quietly make relative, and the staging order are ours (`compass_core::store_bundle`). The C++ `Unzipper` (vendored minizip) is not ported. |
| A local store for the engine's end-to-end tests | `tiny_http` 0.12 (dev only) | **Used**: a real HTTP server on `127.0.0.1:0` serving fixture listings and zips, so the tests go through ureq, the listing parsers and the unpacker exactly as a real install does, without the network. |
| A `null` where the stores' JSON usually has a string | serde's `Option` | **A 5-line `deserialize_with`** in each store model, rather than `serde_with`'s `DefaultOnNull`: one helper, applied to every non-optional field, is less than a new dependency. |

## Phase 5 Track A: the input server and snippet keyword expansion (2026-09-25)

| Need | Crate | Decision |
|---|---|---|
| Read `/dev/input/event*`; create the `/dev/uinput` virtual keyboard | `evdev` 0.13 (safe wrappers over the ioctls; new deps `bitvec`, `funty`, `radium`, `tap`, `wyz`) | **Used** (`compass-input-server::device`). Nodes are opened read-only by us and handed over with `Device::from_fd` (the crate's own `open` tries read-write first); `VirtualDevice::emit` appends a `SYN_REPORT` to every write, so the sink batches the protocol model's events up to its `Sync` and emits the batch, which gives the reader the C++'s exact press–sync–release–sync sequence. No `unsafe`, no hand-written ioctls. |
| Keycode → text, modifier state, the virtual keyboard's character map | `xkbcommon` 0.8 (already locked through `smithay-client-toolkit`), default features off | **Used** (`compass-input-server::keymap`), as the C++ uses libxkbcommon: `key_get_utf8` before `update_key`, `mod_name_is_active(…, STATE_MODS_DEPRESSED)`, and the char map read unshifted then with Left Shift down. |
| Which nodes are keyboards and pointers; hot-plug | `udev` crate (libudev bindings) considered | **Not used.** It would add a build-time `libudev` dependency to every `cargo build --workspace` (CI, the Flatpak SDK) for a lookup of `ID_INPUT_KEYBOARD`/`ID_INPUT_MOUSE`, tags udev's `input_id` builtin computes from the capability bits. `device::is_keyboard`/`is_pointer` apply those tests to the bits `evdev` already reads, and hot-plug is a `notify` (inotify) watch on `/dev/input` — the C++ acts on nothing but "a node appeared" either. Differences: PARITY "Input server" #1. |
| The engine ↔ helper wire | `serde_json` (figura's JSON-RPC shape by hand, ~150 lines) | **Kept byte-compatible with the C++** rather than moved to `compass-ipc`'s postcard: either engine can drive either helper while both ship (PLAN §5), and the NixOS module wraps one path for both. Framing is the existing `compass_core::input_server::frame`/`MessageBuffer` (little-endian, as the C++'s `reinterpret_cast`). |
| Process supervision | `tokio::process` | **Used**, with the existing `RestartPolicy` port. |

## The gaps pass: icons, notifications and the tray (2026-09-25)

| Need | Crate | Decision |
|---|---|---|
| A file's MIME type, for its file-type icon (`renderFileIcon`) | `mime_guess` 2 (already in the tree through `compass-xdg`); `xdg-mime` 0.4 and `tree_magic_mini` 3 considered | **`mime_guess` used; the icon names hand-written** (`compass_ui::icons::names_for_mime`, 12 lines: `/` made `-`, and `<media>-x-generic`, shared-mime-info's defaults). `xdg-mime` reads the system database, globs, magic and `generic-icons` included, which would be exact, but it is unreleased since 2023, adds `nom` 7 and `dirs-next`, and parses the database on the window's first file row. `tree_magic_mini` would sniff extensionless files but reads the same system database at runtime. Both differences are PARITY "The gaps pass, icons and tray". |
