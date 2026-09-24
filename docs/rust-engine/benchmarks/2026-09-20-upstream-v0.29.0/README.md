# First upstream comparison: exploratory, not a cutover gate

Baseline: official upstream **v0.29.0**, commit
`c3415a3ed56676d2960d90975ab319ae8a7aba6e`, unmodified x86_64 AppImage.
Port: `521c0918af82dd206165c7f82fdd31b2a0f6c370`, release build, Rust 1.94.1.
Artifact hashes and exact process roots are in each JSON report.

Three consecutive runs; 100 warmup pairs and 1,000 measured pairs per run.
AB/BA ordering over established sockets; client encode/decode included, CLI spawn
and connection setup excluded. Times below are microseconds.

| Run | Upstream median / p95 / p99 | Rust median / p95 / p99 |
|---|---|---|
| 1 | 37.419 / 71.942 / 202.683 | 17.368 / 38.224 / 82.391 |
| 2 | 20.965 / 36.946 / 50.287 | 8.622 / 19.059 / 25.839 |
| 3 | 20.659 / 40.433 / 54.558 | 8.696 / 17.127 / 21.461 |

**Finding:** this port's warm ping round trip was lower in all three runs.
This is not a search, launch, UI-latency or overall-speed result. The large
between-run spread also makes these observations unsuitable as a CI threshold.
The shared development machine was not CPU-pinned or governor-controlled.

## Memory: recorded, not comparable yet

Median process-tree readings (KiB; 20 samples per engine in each run):

| Run | Upstream RSS / PSS | Rust RSS / PSS |
|---|---|---|
| 1 | 257928 / 212974 | 155860 / 116753 |
| 2 | 260200 / 215277 | 155808 / 116726 |
| 3 | 260200 / 215315 | 155820 / 116754 |

These are **not evidence of a whole-port memory-efficiency win**:

- Xvfb on X11, software rendering, no window manager; not GNOME/Wayland/Flatpak.
- Both launchers were mapped, but upstream also mapped its first-run **Welcome
  to Vicinae** window. Window names were verified with `xdotool`. No screenshot
  assertion was obtained (the installed ImageMagick lacks X11 support).
- The port's daemon **and** separately spawned resident UI were included.
  Upstream's file indexer and input server descendants were included. The
  shared X server and bubblewrap supervisors were excluded on both sides.
- Upstream has running services missing from this port. Neither ran TypeScript
  extensions. The C++ file indexer logged a database-lock error during startup.
- Both saw the same 738-entry real corpus under isolated XDG directories, but
  equal indexed/searchable sets have not been established. No query was timed:
  pristine upstream has no rootQuery endpoint.
- Network access was disabled; home was isolated with bubblewrap. This changes
  background-service behavior relative to a normal session.

## Reproduce and improve

Build and run the [head-to-head harness](../../HEAD-TO-HEAD.md) using the report's
configuration as a template; replace sockets and PIDs with the newly started
processes. The report's `config` object is directly accepted as CONFIG.json.
Raw samples, CPU/kernel/load information and explicit exclusions are retained
in `report-1.json`, `report-2.json` and `report-3.json`.

Before answering the user's full question: finish onboarding, verify equivalent
visible launcher state on the target compositor, establish search parity on a
documented instrumented upstream build, then measure ranking, summon and memory
under equal feature/extension workloads. Preserve these exploratory numbers as
such; do not promote them into a match-or-beat gate.
