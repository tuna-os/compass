# Upstream search inventory audit — 2026-09-20

**Search parity fails before timing.** This is an inventory/response audit, not a
performance result. Neither empty indexes nor different work may establish a win.
The pinned v0.29.0 source plus the minimal `rootQuery` instrumentation compiled
successfully in Release mode. The pristine AppImage remains the default benchmark.

| Query | Upstream results | Rust results | Ordered IDs equal |
|---|---:|---:|---|
| empty | 448 | 452 | no |
| chrom | 3 | 5 | no |
| terminal | 8 | 11 | no |
| calculator | 2 | 3 | no |
| code | 19 | 31 | no |
| é | 105 | 144 | no |
| zzzznonexistent | 0 | 0 | yes |

[report.json](./report.json) records binary/revision/patch identity and every
membership difference. Both engines read the same 738 unmodified real desktop
files, with fresh independent XDG profiles, C.UTF-8 and network disabled. C++
requests the applications provider without disabled entries or a result limit;
Rust's result limit is raised to 10,000 so truncation cannot explain this table.
Qt uses offscreen and Rust runs daemon-only: **no memory or rendering comparison**.

## What explains the mismatch

- The empty-query sets share 407 application IDs. All 45 Rust-only IDs are desktop
  actions. Upstream's `AppRootProvider::loadItems` emits applications, and exposes
  desktop actions in each application's action panel instead.
- Of the 41 upstream-only entries, 40 have `TryExec`; the remaining entry is
  `host--Singular-manual.desktop`, a `Type=Link` URL. Rust's application index
  excludes non-Application entries, and `serve::EngineState::query` filters out
  unresolved `TryExec` entries. These are real policy differences, not corrupted
  corpus files. Host resolution under Flatpak needs its own treatment.
- The daemon still ranks `AppItem`s using `rank_with_frecency`. Their fields include
  GenericName, Comment and Categories. Upstream application root items have an
  empty subtitle and search title, unlocalized title and keywords through the root
  manager. The already-ported `compass-core::root_items` path is not wired into
  this daemon query. The parity ledger already notes the disconnected path; this
  audit demonstrates its end-to-end consequence.

These observations do not license removing actions or hiding entries simply to
make a benchmark pass. Complete the actual root-provider/action-panel integration,
resolve entry visibility and host executable policy, and explicitly account for
upstream's unlocalized-title field. Then rerun this audit and the ordered-ID gate
before publishing search timings. A miss-only workload would conceal the failure.

## Capture again

`capture.py` is the exact container-side probe used for this audit. It uses the
shared process lifecycle/readiness helpers and saves full untimed responses as
`queries.json`. It requires these mounts in an isolated, network-disabled Fedora
container with the upstream build/runtime dependencies:

- `/src`: this checkout, read-only (the port binary was built at the report's revision).
- `/port/vicinae`: the release Rust binary, read-only.
- `/stage/usr`: a staged Release install of upstream commit
  `c3415a3ed56676d2960d90975ab319ae8a7aba6e` with
  `scripts/bench/upstream-v0.29.0-root-query.patch` applied, read-only.
- `/results`: a new empty writable output directory.

Inside that disposable container, install the staged files and run:

```sh
cp -a /stage/usr/. /usr/
python3 /src/docs/rust-engine/benchmarks/2026-09-20-query-audit/capture.py
```

The probe stops and reaps only its own process groups. Do not run its install
command on the host. This is an advanced diagnostic requiring a prepared upstream
build, **not** a claim that the one-command pristine benchmark measures search.
