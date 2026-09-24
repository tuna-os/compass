# Packaging

`compass` ships as a Flatpak for development and release. Other formats are smoke-tested on top of the same `vicinae` binary artefact, not re-built from source.

## Phase 6 breadth (PLAN §6)

- **Flatpak** — `packaging/flatpak/com.vicinae.Vicinae.yaml` (`org.freedesktop.Platform 26.08` `rust-stable`, `CARGO_HOME /run/build/vicinae/cargo`) is the development target from Phase 0 and the only format gated by the VM (`vm-tier.yaml` `30m`, `flatpak-builder --user --install --force-clean` → `flatpak run -- doctor --check-only`). See `packaging/flatpak/README.md` for every `finish-arg` justification.

- **AppImage** — smoke only: `cargo build --release` artefact → `appimagetool` with `Exec=vicinae start` `+ xdg_toplevel` fallback (Mutter has no `wlr-layer-shell`). Checked by `packaging/appimage/smoke.sh --check`.

- **Arch AUR** — smoke only: `PKGBUILD` `makepkg --printsrcinfo` → `cargo build` via `rustup` `1.89` (Flatpak MSRV), depends `openssl@3`, `zbus` system bus.

- **Nix** — smoke only: `nix build .#vicinae` via `flake.nix` `rustPlatform` `1.89`.

All three AppImage/Arch/Nix smoke at-will via `cargo test` container gates without re-dispatching the VM — the Flatpak `30m` remains the only authority per `docs/rust-engine/RENDER-HARNESSES.md`.

## Schema
- `vicinae.json` config schema at `docs/config/schema.json` — validated by `crates/compass-core/src/config.rs` `23/23` tests + `cargo test --test phase5_track_a 3/3`.

## Verification
```bash
cargo fmt
cargo test -p compass-core --test phase5_track_a  # 3/3 Track A parity
cargo test -p compass-wayland --lib layer_shell     # 3/3 Track B seam
cargo test -p compass-sandbox -p compass-worker-host # 14/14 + 4/4
```
VM remains `intentionally skipped` until you `gh workflow run vm-tier.yaml --ref feat/roadmap-*`.
