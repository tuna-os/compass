# Packaging

Every output of the Rust engine installs the same files in the same places. The list is
`scripts/packaging/install-rust-engine.sh`, and every output calls it; `scripts/packaging/smoke.sh`
checks it (PLAN.md §8.6, Suite 5).

| Path under the prefix | What |
|---|---|
| `bin/compass` | the engine and CLI |
| `libexec/compass/compass-sandbox-exec` (Arch: `lib/compass/`) | confines the extension runtime; extensions refuse to run without it |
| `libexec/compass/compass-file-indexer` (Arch: `lib/compass/`) | the file-search helper |
| `libexec/compass/compass-input-server` (Arch: `lib/compass/`) | snippet keyword expansion's keyboard helper; not in the Flatpak (see below) |
| `share/applications/org.tunaos.compass.desktop`, `share/metainfo/…`, `share/icons/hicolor/scalable/apps/…` | desktop entry, AppStream data, icon |
| `share/compass/builtin-icons/*.svg` | the `Icon.*` set extensions draw with |
| `lib/systemd/user/compass.service` | an opt-in user unit (`systemctl --user enable --now compass`); inert in the Flatpak, which does not export units |
| `share/compass/extension-runtime.js` | the extension runtime bundle (`scripts/build-extension-runtime.sh`) |
| `share/compass/compass.schema.json` | the `compass.json` JSON Schema |
| `share/compass/scripts/<name>/` | the first-party Rhai scripts (`extensions/rhai-examples/`), granted what they declare |

The engine finds the helpers, the runtime bundle, the schema and the scripts relative to its own binary, so a
prefix is relocatable. Node is the Flatpak's and the AppImage's own; the Arch and Nix packages use
the distribution's.

## Outputs

| Output | Definition | Built and smoke-tested by |
|---|---|---|
| Flatpak | `flatpak/org.tunaos.compass.yaml` (finish-args justified in `flatpak/README.md`) | `.github/workflows/flatpak.yaml`, on every PR touching it |
| AppImage | `appimage/build-appimage.sh` (linuxdeploy + appimagetool, pinned by checksum) | `.github/workflows/packaging.yaml` |
| Arch | `arch/PKGBUILD` (`compass-git`) | `packaging.yaml`, in an `archlinux:base-devel` container |
| Nix | `nix/compass.nix`, exposed by `flake.nix` as `.#compass` (alias `.#rust-vicinae`) | `packaging.yaml`, with `cachix/install-nix-action` |

`packaging.yaml` runs nightly and on changes to packaging; the Flatpak workflow runs on every PR
that touches the engine. Both end in the same `smoke.sh`: installed layout, `--version`,
`config schema`, `doctor` detecting the missing session, and a started engine answering `ping`.

The flake also exports two modules. `homeManagerModules.default` (`nix/home-manager-module.nix`)
is `programs.compass`: the package, `compass.json` settings, themes, extensions and a
`compass.service` user unit. `nixosModules.default` (`nix/nixos-module.nix`) is
`programs.compass.input-server`, described below. Both accept upstream's `programs.vicinae` names.

The distribution packages install `compass` and `org.tunaos.compass` files only, so they sit beside
upstream's `vicinae` packages rather than conflicting with them.

## The input server

`compass-input-server` (`crates/compass-input-server`) reads every keyboard under `/dev/input` to
notice a snippet keyword being typed, and creates a virtual keyboard through `/dev/uinput` to erase
the keyword and paste the expansion. It opens devices read-only and never grabs them. Both device
paths are root-only by default, so it needs `CAP_DAC_OVERRIDE` — the same model as upstream's
helper. The capability sits on this
one small program, which speaks only JSON-RPC to its parent over stdin and stdout; the launcher
itself runs unprivileged. Adding the user to the `input` group instead would hand every program
they run the keyboard, and still leave `/dev/uinput` closed on most distributions.

| Output | How the helper gets its capability |
|---|---|
| Arch | `arch/compass.install`: `setcap cap_dac_override+ep /usr/lib/compass/compass-input-server` after install and upgrade |
| Nix | the store cannot hold file capabilities. On NixOS, import the flake's `nixosModules.default`: `programs.compass.input-server.enable` (on by default) wraps it with `security.wrappers` and sets `COMPASS_INPUT_SERVER_BIN=/run/wrappers/bin/compass-input-server` for the session |
| AppImage | cannot carry capabilities from inside the image; run `sudo setcap cap_dac_override+ep` on an extracted copy and point `COMPASS_INPUT_SERVER_BIN` at it |
| Flatpak | **not shipped.** The sandbox has no `/dev/input` or `/dev/uinput` and no finish-arg grants them (`--device=all` exposes `/dev` nodes but the helper would still need a capability no Flatpak can hold). The engine does not start it there, and `compass doctor` says so; snippets are still copied and pasted from the launcher |
| From source | `cargo build -p compass-input-server`, then `sudo setcap cap_dac_override+ep target/debug/compass-input-server` |

`compass doctor` reports whether the helper is found, whether it carries the capability (via
`getcap`), and what the running engine says about it; `compass input-server status|enable|disable`
reads and flips `input_server.enabled`.

## Configuration schema

`schema/compass.schema.json` is generated from `compass_core::config` by `schemars` and never edited
by hand. The `config_schema` test fails when it drifts from the types; regenerate it with

```sh
COMPASS_UPDATE_SCHEMA=1 cargo test -p compass-core --test config_schema
```

`scripts/packaging/check-config-schema.py` validates it, `schema/example.compass.json` and a set of
negative cases with the `jsonschema` package, which is what an editor would do with it.

Upstream Vicinae's `settings.json` is migrated with `compass config migrate` (dry run) and
`compass config migrate --write`; until a `compass.json` exists the engine migrates it in memory at
every start. See `crates/compass-core/src/config_migration.rs` for what maps and what is reported
as left behind.
