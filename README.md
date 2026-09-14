<div align="center">
  <img width="112" src="extra/compass.svg" alt="Compass logo" />
  <h1>Compass</h1>
  <p><strong>A fast, extensible command palette for Linux—being rebuilt in Rust.</strong></p>
  <p>
    <a href="https://github.com/tuna-os/compass/actions/workflows/rust.yaml"><img src="https://github.com/tuna-os/compass/actions/workflows/rust.yaml/badge.svg" alt="Rust CI"></a>
    <a href="https://github.com/tuna-os/compass/actions/workflows/flatpak.yaml"><img src="https://github.com/tuna-os/compass/actions/workflows/flatpak.yaml/badge.svg" alt="Flatpak CI"></a>
    <a href="LICENSE"><img src="https://img.shields.io/github/license/tuna-os/compass" alt="GPL-3.0 license"></a>
  </p>
</div>

![Compass launcher](docs/screenshots/launcher.png)

<p align="center"><em>The feature-complete launcher UI. The Rust UI is replacing it incrementally behind parity gates.</em></p>

Compass puts applications, commands, clipboard history, snippets, files, calculations, emoji,
windows and extensions behind one keyboard-first interface. It is a hard fork of
[Vicinae](https://github.com/vicinaehq/vicinae), with the Linux engine and UI being migrated from
C++/Qt to a modular Rust workspace.

## Migration status

The Rust port is active and **not yet the default engine**. `main` currently carries both engines so
behaviour can be compared before cutover. The public project name is Compass; the `vicinae` binary,
Flatpak ID, socket, config paths and extension API names remain temporarily compatible with existing
installations. See [ADR-0012](docs/rust-engine/adr/0012-compass-public-brand.md).

- [Transformation plan](docs/rust-engine/PLAN.md)
- [Parity ledger](docs/rust-engine/PARITY.md)
- [Roadmap epic](https://github.com/tuna-os/compass/issues/2)
- [Architecture decisions](docs/rust-engine/adr/README.md)

The current Rust vertical slice opens a native Iced launcher, indexes desktop applications, ranks
them with the ported fuzzy-search semantics, launches the selected result, exposes IPC and
diagnostics, and is exercised in Flatpak and Bluefin VM CI. Clipboard, extension-host and builtin
feature parity are still in progress; the parity ledger is the source of truth.

## Try the Rust launcher

Install the pinned Rust toolchain, then run:

```sh
cargo run -p vicinae -- ui
```

The launcher currently targets a graphical Linux session. To run the same checks as Rust CI:

```sh
make check-rust
```

For the Bluefin/Flatpak development path, install `flatpak-builder` and the Freedesktop SDK, then:

```sh
make flatpak-rust
flatpak run com.vicinae.Vicinae -- ui
```

The legacy identifiers in those commands are intentional migration compatibility, not the public
brand.

## Architecture

The workspace separates desktop-entry parsing, search, application state, IPC, platform services,
Wayland/portal integration, GNOME Shell integration, UI and extension APIs into `compass-*` crates.
The `vicinae` package is the compatibility CLI and binary while the cutover is underway.

New Rust code must pass formatting, Clippy with warnings denied, workspace tests, doctests, minimum
supported Rust, scorer/crypto parity, Flatpak source checks and the platform build matrix before it
is merged.

## Contributing

Start with [CONTRIBUTING.md](CONTRIBUTING.md) and the open
[roadmap issues](https://github.com/tuna-os/compass/issues). Port work should preserve observable
behaviour or update the parity ledger with evidence for an intentional difference. A C++ test may
only be removed in the same change that adds its Rust replacement.

## Project history

Compass is derived from Vicinae and remains grateful to its maintainers, contributors and sponsors.
The inherited C++ engine and TypeScript extension ecosystem are the behavioural reference during
the migration. Special thanks also go to the
[Soulver](https://soulver.app) team for allowing the project to ship SoulverCore as an optional
calculator backend on macOS.
