# ADR-0020: The Phase 7 rebrand

**Status:** Accepted · **Date:** 2026-09-25 · Supersedes: ADR-0012's list of compatibility identifiers · Relates to: ADR-0007, ADR-0017, `CUTOVER.md`

## Context

[ADR-0012](./0012-compass-public-brand.md) made Compass the public name but kept every technical
identifier as `vicinae`: the executable, the `com.vicinae.Vicinae` application ID, the socket and
config paths, D-Bus names, URL schemes and the `@vicinae/api` package. It deferred the change to
the Phase 7 cutover, on the condition that cutover supply aliases, a tested migration and a
rollback.

Keeping those names had a cost. A user who installs "Compass" types `vicinae`, finds their
configuration under `~/.config/vicinae`, and cannot install Compass next to Vicinae because both
provide `/usr/bin/vicinae`. [ADR-0017](./0017-a-new-launcher-not-a-reimplementation.md) already
decided that Compass imports Vicinae's data rather than sharing it, which removes the main reason
to share its names.

## Decision

**Every identifier Compass owns is renamed.**

| What | Was | Now |
|---|---|---|
| Flatpak and application ID, desktop file, metainfo, icon | `com.vicinae.Vicinae` | `org.tunaos.compass` |
| Executable | `vicinae` | `compass` |
| Rust binary crate | `crates/vicinae`, package `vicinae` | `crates/compass`, package `compass` |
| Config directory | `$XDG_CONFIG_HOME/vicinae` | `$XDG_CONFIG_HOME/compass` |
| Data, cache and state directories | `…/vicinae` | `…/compass` |
| Runtime directory and IPC socket | `$XDG_RUNTIME_DIR/vicinae…` | `$XDG_RUNTIME_DIR/compass…` |
| Environment variables | `VICINAE_*` | `COMPASS_*` |
| D-Bus names, interfaces and object paths | `com.vicinae.*`, `/com/vicinae/*` | `org.tunaos.compass.*`, `/org/tunaos/compass/*` |
| URL scheme Compass emits | `vicinae://` | `compass://` |
| GNOME Shell extension UUID | Vicinae's | `compass@tunaos.org` |
| systemd user unit | `vicinae.service` | `compass.service` |
| Product name in any user-facing text | Vicinae | Compass |

**What keeps the `vicinae` name, and why.** Nothing else may keep it.

- **The extension SDK module `@vicinae/api`**, like `@raycast/api`. Third-party extensions import
  it by that name, and renaming it would break every one of them. A `@compass/api` alias is
  allowed, not required.
- **`vicinae://` deeplinks are still accepted**, alongside `compass://` and `raycast://`.
  Extensions and the Vicinae Store emit them. Compass itself emits `compass://`.
- **The Vicinae Store**: extension IDs such as `store.vicinae.<name>`, its name and its API host.
  It is an upstream third-party service, named the way "Raycast Store" is.
- **`VICINAE_*` environment variables are read as a fallback** when the `COMPASS_*` one is unset,
  with a one-time deprecation message in the log, so existing scripts and unit files keep working.
- **Credit to upstream Vicinae**: the README, LICENSE, documentation history, the parity ledger and
  ADR prose describing the C++ reference, and the pinned upstream AppImage that the head-to-head
  benchmarks run against.
- **The legacy C++ tree** (`src/server`, `src/cli`, `src/lib`, `src/proto` and the rest) is not
  renamed. It is a behavioural reference and is not built into any Compass package
  ([ADR-0013](./0013-qt-leaves-the-repository.md)).

Historical ADRs keep their wording. Where they describe current behaviour, this ADR takes
precedence.

**Migration.** On startup, the engine checks each of the config, data, cache and state
directories. If the `compass` directory does not exist and the `vicinae` one does, it renames
`vicinae` to `compass` and leaves a `vicinae` symlink pointing at `compass`, so rolling back to an
older build still finds the same data. An existing `compass` directory is never overwritten. The
engine logs what it moved. The tests use temporary XDG directories only, never the real `$HOME`.

**No Flatpak data migration.** A Flatpak keeps an app's data under `~/.var/app/<app-id>`, so a new
ID would normally strand the old one's data. The `com.vicinae.Vicinae` build was on the TunaOS
remote for a few minutes at most before the rename, so no installed base needs to be carried
across. Anyone who did install it can copy `~/.var/app/com.vicinae.Vicinae/config/vicinae` into
`~/.var/app/org.tunaos.compass/config/`, and the migration above renames it.

## Consequences

- Compass installs as `/usr/bin/compass` and no longer conflicts with an installed Vicinae package.
- A Vicinae user's configuration, data and extensions carry over on first start. After that,
  Vicinae and Compass share one directory through the symlink. Running both against it is not
  supported.
- The Flatpak permissions that name the old ID, such as portal grants and the GlobalShortcuts
  binding, are asked for again under `org.tunaos.compass`.
- Scripts that call `vicinae`, bind `vicinae toggle` or set `VICINAE_*` need updating. The
  environment fallback hides the last of these for now. The first two fail loudly.
- The search for `vicinae` across the tree (the C++ tree excluded) should find only the items in
  the list above. Anything else is a bug.

## What would change this

Losing the upstream store or SDK as a compatibility target would let `@vicinae/api` and the
`vicinae://` scheme go. The environment variable fallback can be removed after one release that
logs the deprecation.
