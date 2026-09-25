# Packaging

Every output of the Rust engine installs the same files in the same places. The list is
`scripts/packaging/install-rust-engine.sh`, and every output calls it; `scripts/packaging/smoke.sh`
checks it (PLAN.md §8.6, Suite 5).

| Path under the prefix | What |
|---|---|
| `bin/vicinae` | the engine and CLI |
| `libexec/vicinae/compass-sandbox-exec` (Arch: `lib/vicinae/`) | confines the extension runtime; extensions refuse to run without it |
| `libexec/vicinae/vicinae-file-indexer` (Arch: `lib/vicinae/`) | the file-search helper |
| `libexec/vicinae/vicinae-input-server` (Arch: `lib/vicinae/`) | snippet keyword expansion's keyboard helper; not in the Flatpak (see below) |
| `share/applications/com.vicinae.Vicinae.desktop`, `share/metainfo/…`, `share/icons/hicolor/scalable/apps/…` | desktop entry, AppStream data, icon |
| `share/vicinae/builtin-icons/*.svg` | the `Icon.*` set extensions draw with |
| `share/vicinae/extension-runtime.js` | the extension runtime bundle (`scripts/build-extension-runtime.sh`) |
| `share/vicinae/vicinae.schema.json` | the `vicinae.json` JSON Schema |
| `share/compass/scripts/<name>/` | the first-party Rhai scripts (`extensions/rhai-examples/`), granted what they declare |

The engine finds the helpers, the runtime bundle, the schema and the scripts relative to its own binary, so a
prefix is relocatable. Node is the Flatpak's and the AppImage's own; the Arch and Nix packages use
the distribution's.

## Outputs

| Output | Definition | Built and smoke-tested by |
|---|---|---|
| Flatpak | `flatpak/com.vicinae.Vicinae.yaml` (finish-args justified in `flatpak/README.md`) | `.github/workflows/flatpak.yaml`, on every PR touching it |
| AppImage | `appimage/build-appimage.sh` (linuxdeploy + appimagetool, pinned by checksum) | `.github/workflows/packaging.yaml` |
| Arch | `arch/PKGBUILD` (`compass-git`) | `packaging.yaml`, in an `archlinux:base-devel` container |
| Nix | `nix/compass.nix`, exposed by `flake.nix` as `.#compass` (alias `.#rust-vicinae`) | `packaging.yaml`, with `cachix/install-nix-action` |

`packaging.yaml` runs nightly and on changes to packaging; the Flatpak workflow runs on every PR
that touches the engine. Both end in the same `smoke.sh`: installed layout, `--version`,
`config schema`, `doctor` detecting the missing session, and a started engine answering `ping`.

All three distribution packages conflict with the C++ engine's, which also installs
`/usr/bin/vicinae`, until the Phase 7 dispatcher exists (PLAN.md §5).

## The input server

`vicinae-input-server` (`crates/compass-input-server`) reads every keyboard under `/dev/input` to
notice a snippet keyword being typed, and creates a virtual keyboard through `/dev/uinput` to erase
the keyword and paste the expansion. It opens devices read-only and never grabs them. Both device
paths are root-only by default, so it needs `CAP_DAC_OVERRIDE` — the same model as the C++ helper,
which `make postbuild` and the NixOS module's `security.wrappers` grant. The capability sits on this
one small program, which speaks only JSON-RPC to its parent over stdin and stdout; the launcher
itself runs unprivileged. Adding the user to the `input` group instead would hand every program
they run the keyboard, and still leave `/dev/uinput` closed on most distributions.

| Output | How the helper gets its capability |
|---|---|
| Arch | `arch/compass.install`: `setcap cap_dac_override+ep /usr/lib/vicinae/vicinae-input-server` after install and upgrade |
| Nix | the store cannot hold file capabilities. On NixOS, wrap it as the C++ module does: `security.wrappers.vicinae-input-server = { source = "${compass}/libexec/vicinae/vicinae-input-server"; capabilities = "cap_dac_override+ep"; owner = "root"; group = "root"; }`, and set `VICINAE_INPUT_SERVER_BIN=/run/wrappers/bin/vicinae-input-server` for the engine |
| AppImage | cannot carry capabilities from inside the image; run `sudo setcap cap_dac_override+ep` on an extracted copy and point `VICINAE_INPUT_SERVER_BIN` at it |
| Flatpak | **not shipped.** The sandbox has no `/dev/input` or `/dev/uinput` and no finish-arg grants them (`--device=all` exposes `/dev` nodes but the helper would still need a capability no Flatpak can hold). The engine does not start it there, and `vicinae doctor` says so; snippets are still copied and pasted from the launcher |
| From source | `cargo build -p compass-input-server`, then `sudo setcap cap_dac_override+ep target/debug/vicinae-input-server` |

`vicinae doctor` reports whether the helper is found, whether it carries the capability (via
`getcap`), and what the running engine says about it; `vicinae input-server status|enable|disable`
reads and flips `input_server.enabled`.

## Configuration schema

`schema/vicinae.schema.json` is generated from `compass_core::config` by `schemars` and never edited
by hand. The `config_schema` test fails when it drifts from the types; regenerate it with

```sh
COMPASS_UPDATE_SCHEMA=1 cargo test -p compass-core --test config_schema
```

`scripts/packaging/check-config-schema.py` validates it, `schema/example.vicinae.json` and a set of
negative cases with the `jsonschema` package, which is what an editor would do with it.

The C++ engine's `settings.json` is migrated with `vicinae config migrate` (dry run) and
`vicinae config migrate --write`; until a `vicinae.json` exists the engine migrates it in memory at
every start. See `crates/compass-core/src/config_migration.rs` for what maps and what is reported
as left behind.
