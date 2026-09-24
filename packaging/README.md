# Packaging

Every output of the Rust engine installs the same files in the same places. The list is
`scripts/packaging/install-rust-engine.sh`, and every output calls it; `scripts/packaging/smoke.sh`
checks it (PLAN.md §8.6, Suite 5).

| Path under the prefix | What |
|---|---|
| `bin/vicinae` | the engine and CLI |
| `libexec/vicinae/compass-sandbox-exec` (Arch: `lib/vicinae/`) | confines the extension runtime; extensions refuse to run without it |
| `libexec/vicinae/vicinae-file-indexer` (Arch: `lib/vicinae/`) | the file-search helper |
| `share/applications/com.vicinae.Vicinae.desktop`, `share/metainfo/…`, `share/icons/hicolor/scalable/apps/…` | desktop entry, AppStream data, icon |
| `share/vicinae/builtin-icons/*.svg` | the `Icon.*` set extensions draw with |
| `share/vicinae/extension-runtime.js` | the extension runtime bundle (`scripts/build-extension-runtime.sh`) |
| `share/vicinae/vicinae.schema.json` | the `vicinae.json` JSON Schema |

The engine finds the helpers, the runtime bundle and the schema relative to its own binary, so a
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
