# Flatpak packaging

Bluefin is the first target and has no traditional package manager, so the Flatpak is the
**development** target from Phase 0 rather than a packaging step at the end. See
[`../../docs/rust-engine/PLAN.md`](../../docs/rust-engine/PLAN.md) §3.6.

## Status

The manifest is built on pull requests and the bundle is exercised in the Bluefin VM tier. That
tier verifies the Wayland session, portals, sandbox nesting and that the Rust launcher paints. The
nightly VM remains the authoritative target integration check; syntax validation alone is not
treated as packaging evidence.

## Building

After installation, open **Compass** from the application grid. Its `start`
entrypoint reuses a running launcher or starts an engine and a resident window
together. The engine requests the native global-shortcut permission; Escape
hides the launcher so the granted shortcut can bring it back.

This is not yet the complete first-run experience (#154): start-at-login is
not enabled automatically, and onboarding, permission-refusal guidance and
an explicit login-start choice remain to be implemented. Closing the graphical
session stops an engine it started itself, but does not stop an independently
managed engine. The development `ui` and `serve` commands remain available.

```sh
# One-off: the offline dependency manifest Flathub builds require.
python3 flatpak-cargo-generator.py ../../Cargo.lock -o cargo-sources.json

flatpak-builder --user --install --force-clean build com.vicinae.Vicinae.yaml
flatpak run com.vicinae.Vicinae -- doctor --check-only
```

`flatpak-cargo-generator.py` comes from
[flatpak-builder-tools](https://github.com/flatpak/flatpak-builder-tools). It is why `Cargo.lock`
must stay committed: Flathub builds have no network access, so every dependency is declared up
front.

## Why each permission exists

Flathub review asks for this, and "we needed it" is not a reason a reviewer can check. Do not add a
`finish-arg` without adding its justification here.

| Permission | Why | Could we drop it? |
|---|---|---|
| `--socket=wayland` | The engine draws a Wayland surface. | No. |
| `--device=dri` | Iced renders through wgpu. Without GPU access it falls back to software rendering, if it starts at all. | No. |
| `--own-name=com.vicinae.Vicinae` | Our own bus name, for single-instance and CLI activation. | No. |
| `--talk-name=org.freedesktop.portal.Desktop` | GlobalShortcuts (the hotkey mechanism on GNOME), OpenURI, FileChooser, Screenshot. | No. |
| `--talk-name=org.gnome.Shell.Extensions.Vicinae` | On GNOME 50/51 the Shell extension is the **only** mechanism for window switching, clipboard history and paste — Mutter implements none of the relevant protocols and `org.gnome.Shell.Introspect` is allowlisted to the portal backends. See [REFERENCES.md §3](../../docs/rust-engine/REFERENCES.md). | Only if GNOME ships a portal for window listing. |
| `--talk-name=org.freedesktop.Flatpak` | `flatpak-spawn --host`, to launch host applications. | No — but see below. |
| `--filesystem=host-os:ro` and the `applications`/`icons` paths | Indexing the application catalogue: what exists, what it is called, what its icon is. Read-only; we never write to any of them. | No, though the list could be trimmed if we dropped Homebrew or system Flatpak discovery. |

### The one that deserves scrutiny

`--talk-name=org.freedesktop.Flatpak` grants `flatpak-spawn --host`, which is **equivalent to running
outside the sandbox**. A launcher that cannot launch host applications is not a launcher, so this is
unavoidable for the category — but it should be stated plainly in review rather than buried in a
list, and it is worth remembering that the Flatpak boundary provides us much less isolation than the
manifest's other entries suggest.

It also means the extension-worker sandbox in Phase 4 (Landlock + seccomp) is doing real work: it is
not redundant with the Flatpak sandbox, because the Flatpak sandbox is this porous by necessity.

### Deliberately absent

- **`--socket=fallback-x11`.** The engine is Wayland-only by design. Claiming an X11 socket we never
  use is asking for a permission we do not need. (The original spec had it; we do not.)
- **`--socket=pulseaudio`.** The spec included it for "media extensions". We have no such feature
  yet. Add it when something needs it, not before.
- **`--filesystem=~/.local/share/gnome-shell/extensions`.** Deliberately refused — see
  [ADR-0004](../../docs/rust-engine/adr/0004-gnome-shell-extension-distribution.md). We never install
  the Shell extension ourselves; the user installs it from extensions.gnome.org, or it arrives baked
  into the Bluefin image.
- **`org.kde.Platform`.** The original spec specified the KDE runtime, which made sense for a Qt
  application. The Rust engine needs neither Qt nor GTK at runtime, so `org.freedesktop.Platform` is
  smaller, faster to download, and a smaller attack surface.
