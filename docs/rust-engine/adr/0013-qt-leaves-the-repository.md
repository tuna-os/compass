# ADR-0013: Qt leaves the repository; Linux-first is a sequence, not a scope limit

**Status:** Accepted · **Date:** 2026-09-17 · Supersedes: ADR-0007 decision 3 · Relates to: PLAN.md §6 Phase 7–8

## Context

ADR-0007 decision 3 said the Rust engine targets Linux and that "macOS and Windows continue to ship
the C++ build until a follow-up project". Its consequences section drew the right conclusion —
"Phase 8's deletion is Linux targets only" — but nobody had costed what that leaves behind.

Counting `.cpp`/`.hpp` under `src/server` by path:

| | files |
|---|---:|
| total | 840 |
| Linux-specific | 64 |
| macOS-specific | 28 |
| Windows-specific | 70 |
| cross-platform | 678 |

So the recorded scope deletes on the order of **64** files and keeps **~776 Qt files** indefinitely.

> **Corrected after this ADR was accepted.** The table above was produced by matching path
> fragments, and both its method and its numbers are wrong. It counts `src/server/src/ui/windows` —
> the UI's *window* classes — as Windows code, and reports 28 "macOS files" where only **three**
> macOS translation units exist. Attributing each unit to the CMake block that lists it gives
> **59 Linux / 33 Windows / 3 macOS / 229 shared** `.cpp`.
>
> More importantly, the table asks the wrong question. Platform behaviour is mostly *not* in
> platform files: there are **102 `Q_OS_MAC` sites across 38 shared files, 100 `Q_OS_WIN` across 43,
> and 72 `Q_OS_LINUX` across 30**, with 61 shared files carrying at least one. That is what
> Phases 9 and 10 actually have to resolve, and it is why "delete a platform's share of the
> cross-platform core" — which this ADR's Phase 9 sketch implied — is not executable.
>
> The decision this ADR records is unaffected: Qt leaves, Linux-first is a sequence, and the seam
> comes before Phase 4. Only the sizing was wrong. [PLAN.md](../PLAN.md) §6 carries the measured
> figures.
The bulk is the Qt UI layer, the builtins and the extension model. Two consequences follow that the
old wording did not state: the repository stays majority C++/Qt, and every change to shared
behaviour is made twice for as long as that lasts.

The project owner has since said the intent is to move off Qt as part of the Rust migration. Under
ADR-0007 decision 3 that does not happen — not late, but never, because "a follow-up project" has
no owner, no phase and no gate.

**What makes this decidable now rather than later** is a structural fact about the Rust workspace,
measured rather than assumed:

- Linux-only dependencies (`zbus`, `ashpd`, `wayland-client`) are confined to four crates —
  `compass-portals`, `compass-shell`, `compass-wayland`, and the `vicinae` binary. They are not
  smeared across the workspace.
- There are **zero** `cfg(target_os)` guards anywhere. Platform variance lives in separate crates
  rather than in conditional compilation, which is the cleaner arrangement.
- The renderer is already portable: `iced` + `winit` + `wgpu`, no Qt in any crate manifest.

But:

- **`compass-platform` defines no traits.** It is named like a platform seam and is not one: two
  files, and it *depends on* `compass-portals`, so it is a Linux implementation wearing the name of
  an abstraction.
- **`compass-ui` declares `compass-portals` and `compass-wayland` as dependencies.** *Corrected
  after this ADR was accepted:* the first draft said the crate was "Linux-bound by its dependency
  edges", which overstates it. Those two dependencies are **unused** — the only mention of either
  in `crates/compass-ui/src` is a doc comment, and the crate builds clean with both removed
  (verified, not assumed). They were real edges and they did bind the build, but removing them is
  deleting two lines, not untangling code.

  The coupling that *is* real sits one level down: `compass-ui` → `compass-platform` →
  `compass-portals`.

`compass-platform`'s own doc comment claims it handles "launching applications, file indexing,
clipboard". It handles launching. The other two do not exist.

And `launch.rs` is not a portable API with a Linux backend — it *is* the Linux backend, sitting in
the crate named for the abstraction: `flatpak-spawn --host`, then the XDG `OpenURI` portal, then a
direct spawn.

Every phase that lands before that seam exists makes the macOS and Windows work more expensive,
because Phase 4's extension host and Phase 5's breadth will be written against whatever shape
`compass-ui` and `compass-platform` have at the time.

## Decision

**1. Qt leaves the repository.** The end state is a Rust-primary repository with no Qt on any
platform. This is the goal the migration is now working toward, replacing "until a follow-up
project".

**2. Linux-first remains — as a sequence, not a scope limit.** Nothing about the Linux ordering
changes. Phases 0–7 are untouched, Linux still cuts over first, and macOS/Windows still ship the
C++ engine until their own phases land. The change is that those phases exist and are owned rather
than being deferred to an unowned successor.

**3. Phase 8 deletes Linux targets only, and is renamed to say so.** It is not "Removal"; it is the
removal of the Linux C++ engine. The repository is not Rust-primary at that point and the plan no
longer claims it is.

**4. Phases 9 and 10 are added: macOS and Windows.** Each migrates its platform services and
deletes its C++ targets. Qt leaves when Phase 10 completes.

**5. The platform seam is built now, before Phase 4.** Concretely, and checkably:

- `compass-platform` gains the traits for platform services — clipboard, window management, tray,
  global shortcuts, file indexing — and stops depending on `compass-portals`.
- `compass-ui` depends on `compass-platform`, not on `compass-portals` or `compass-wayland`
  directly. (Already done, and it cost two deleted lines — see the correction above.)
- The Linux crates become implementations selected at composition, in the `vicinae` binary.

This is the only part of this ADR that changes work in the near term, and it is deliberately small:
it is a dependency-direction fix, not a rewrite. Doing it now costs days; retrofitting it after
Phases 4–6 costs weeks and touches code nobody wants to touch twice.

## Consequences

- The timeline grows by two phases. They are honestly sized rather than hidden: 98 platform-specific
  files plus macOS and Windows implementations for the services behind the new seam.
- Until Phase 10, the repository still carries two engines and shared behaviour is still changed
  twice. That cost is unchanged by this ADR — what changes is that it now ends.
- Phase 5's second-compositor work and Phases 9–10 become the same kind of task: implement the seam
  for another platform. Done in that order, wlroots is a rehearsal for macOS.
- A seam with one implementation is untested as an abstraction. The mock-bus suite pattern from
  `compass-shell` applies: a second implementation, even a fake one, is what proves the trait is a
  trait and not a description of Linux.
- ADR-0007 decision 3 is superseded. Decisions 1, 2 and 4 stand — 2 having already been superseded
  by ADR-0012.
