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

## Install

> **There is no tagged release and Compass is not on Flathub yet.** Every path below builds or
> installs a development build. The Rust port is [not yet the default engine](#migration-status),
> so expect a launcher that opens, searches applications and launches them — not feature parity
> with the screenshot above.

**Requirements.** A Wayland session; GNOME is the first target and the only one covered by CI.
There is no X11 fallback — the engine is Wayland-only by design. The Flatpak paths also need
`flatpak` and access to Flathub for the `org.freedesktop.Platform//26.08` runtime.

### From a CI build

The quickest way to get a binary. Every successful run of the
[Flatpak workflow](https://github.com/tuna-os/compass/actions/workflows/flatpak.yaml) uploads a
single-file bundle as the `flatpak-bundle` artifact. Artifacts expire after 14 days and downloading
them requires being signed in to GitHub.

With the [`gh` CLI](https://cli.github.com):

```sh
run=$(gh run list --repo tuna-os/compass --workflow flatpak.yaml \
        --branch main --status success --limit 1 --json databaseId --jq '.[0].databaseId')
gh run download "$run" --repo tuna-os/compass --name flatpak-bundle

flatpak remote-add --if-not-exists --user flathub https://flathub.org/repo/flathub.flatpakrepo
flatpak install --user --bundle com.vicinae.Vicinae.flatpak
```

Or download `flatpak-bundle` from a run page in a browser, unzip it, and run the last two commands.

The bundle carries the application only, so the runtime still has to come from somewhere — that is
what the `remote-add` line is for. Once both are installed nothing further needs the network.

### Build the Flatpak yourself

Install `flatpak-builder` and the Freedesktop SDK, then:

```sh
make flatpak-rust
```

This builds and installs into your user installation. See
[`packaging/flatpak/README.md`](packaging/flatpak/README.md) for what each sandbox permission in the
manifest is for and why.

### From source

Cargo builds it without any Flatpak involved. The toolchain is pinned in `rust-toolchain.toml`, so
[rustup](https://rustup.rs) will fetch the right one on first build:

```sh
cargo run -p vicinae -- ui
```

`ui` opens the launcher in the foreground and indexes and ranks in-process — it does not need an
engine running alongside it.

## Running it

```sh
flatpak run com.vicinae.Vicinae -- ui       # open the launcher
flatpak run com.vicinae.Vicinae -- doctor   # what works on this machine, and what does not
```

Start with `doctor` if something misbehaves: it reports the session type, bus, portals and index
state, and `doctor --check-only` exits non-zero when a check fails.

For the resident mode the launcher is moving to, run the engine and let a window attach to it
([ADR-0015](docs/rust-engine/adr/0015-the-launcher-window-is-resident.md)):

```sh
flatpak run com.vicinae.Vicinae -- serve    # then, from anywhere:
flatpak run com.vicinae.Vicinae -- toggle
```

`serve` asks the GlobalShortcuts portal for `LOGO+space`, which on GNOME means a permission prompt.
Pass `serve --no-hotkey` if your compositor already binds a key to `toggle`, or if there is no
GlobalShortcuts backend.

The legacy `vicinae` and `com.vicinae.Vicinae` identifiers in these commands are intentional
migration compatibility, not the public brand.

### Hacking on it

To run the same checks as Rust CI:

```sh
make check-rust
```

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
