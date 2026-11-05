# Performance regression gates

§8.8 asks for Suite 4 to be "every PR informational; **blocking on `main`**". #127 asks for a
per-commit idle-RSS gate, and #310 for comparable actions to be no worse than upstream Vicinae. All
three use one comparator, [`scripts/ci/perf-gate.py`](../../scripts/ci/perf-gate.py), and one
workflow, [`perf.yaml`](../../.github/workflows/perf.yaml) (#306, #307, #310).

## What is measured, and under which name

| metric | statistic | where it comes from | gated |
|---|---|---|---|
| `slas.fuzzy_rank_top20_of_10k` | criterion median, ns | `perf.yaml`, `criterion-report.py --points` | yes, on `main` |
| `slas.ipc_roundtrip_ping` | criterion median, ns | the same | yes, on `main` |
| `engine.idle_rss_kb` | median of 3 starts, kB, debug | `perf.yaml`, `crates/compass/tests/idle_memory.rs` | yes, on `main` |
| `engine.idle_hwm_kb` | median of 3 starts, kB, debug | the same | yes, on `main` |
| `engine.idle_rss_kb` (profile `flatpak`) | one reading, kB | VM tier, `launcher-rss` | recorded only |
| `window.idle_rss_kb` (renderer `llvmpipe`) | one reading, kB | VM tier, `launcher-rss` | recorded only |
| `window.idle_rss_kb` (a hardware renderer) | one reading, kB | `perf-gate.py measure-rss`, by hand | **not yet recorded** |
| `versus.*`, five actions | Compass's and upstream's medians from one run | `perf.yaml`, `scripts/bench/compare.py` | yes, on `main`, against upstream ([below](#against-upstream-vicinae)) |

The engine and the resident window process have separate names. The engine draws nothing, so its
figure means the same thing in a VM and on a real machine. The window's figure is mostly the
renderer: it is about 135 MB under llvmpipe, where wgpu's software rasteriser lives inside the
process. A threshold set from that figure would gate Mesa.

The series stores criterion's **median**, not the mean that criterion prints. On the IPC row the two
statistics differ by about 6%, so a series has to use one of them throughout.

## A point

Each figure is one line of JSON. The measuring step writes the metric, the statistic, the unit and
the value. It also writes the parts of the environment that only it knows: the build profile, the
architecture and the renderer. `perf-gate.py stamp` then adds the rest on the same machine: the runner
label, the image (`ImageOS`), the image version, the CPU model from `/proc/cpuinfo`, the core count,
the head SHA, the ref, the event, the run id and the time. A point that lacks any of these is
rejected.

Two points are compared only when `runner`, `image`, `cpu`, `cores`, `arch`, `profile` and `renderer`
all match. The image version is recorded but is not part of the key. The hosted image is rebuilt
every week, so keying on it would keep every series too short to judge.

## Where the baseline comes from

`perf.yaml` runs on every pull request, on every push to `main`, and **every six hours on `main`**.
Each run measures all four gated figures in one job, on one machine, and uploads them as
`perf-points-<head sha>` with 90-day retention. The `perf-gate` job reads this run's points. It then
runs `perf-gate.py fetch`, which uses `gh run download` with `actions: read` to collect `perf-points-*`
from up to 150 earlier `perf.yaml` runs on `main`, from pushes and from the schedule. Cancelled runs
are skipped.

The schedule exists because pushes alone are too slow. `main` had 26 pushes in October 2026, spread
over at least two CPU models, and a series needs 30 points per model. A scheduled run measures code
that has already been measured, on whatever machine the pool provides. That is the runner-to-runner
spread on unchanged code, which is the spread the tolerance should come from. ADR-0022 §5 says "the
tolerance is derived from repeated runs (#306), not bought with quieter hardware". At four runs a
day, a CPU model that serves 70% of runs fills its window in about eleven days. A model that serves
30% fills it in about 25 days.

The baseline comes from artifacts, not from the actions cache. A cache entry is evicted after seven
days without use, and an eviction would silently restart every series. An artifact is kept for its
stated retention, and it belongs to the run and the commit that produced it.

## The tolerance, and the analysis behind it

For each metric, the gate takes the last **30** compatible points from `main`. It leaves out points
from the commit being judged. With fewer than 30 points it reports the count and does not judge. With
30, a figure has regressed when it is above

$$\text{median} + \max(4 \times 1.4826 \times \text{MAD},\ 1\% \times \text{median})$$

MAD is the median absolute deviation of the window, and $1.4826 \times \text{MAD}$ estimates the
standard deviation of the noise while ignoring outliers. Only an increase counts, because lower is
better for every metric here.

The constants come from replaying the history that existed before the gate.
[`benchmarks/2026-11-05-suite4-main-series/`](./benchmarks/2026-11-05-suite4-main-series/) holds the
criterion medians of all 122 pushes to `main` between 2026-09-20 and 2026-11-05 whose
`slas-criterion-*` artifacts were still retained, with each run's region and image version taken from
its log. Each table below comes from one command:

```sh
dir=docs/rust-engine/benchmarks/2026-11-05-suite4-main-series
python3 scripts/ci/perf-gate.py analyse $dir/series.jsonl                  # 1
python3 scripts/ci/perf-gate.py analyse $dir/series-split-by-mode.jsonl    # 2
for w in 10 15 20 30; do python3 scripts/ci/perf-gate.py analyse $dir/series-step-free.jsonl --window $w; done  # 3
```

**1. The hosted pool is not one machine.** The fuzzy row's medians form two clusters, with nothing
between them: 36 runs around 620 µs, up to 761 µs, and 86 runs around 904 µs, from 804 µs. The IPC
row splits the same way, at about 23 µs and 34 µs. The CPU model was not recorded, so the cause is an
inference. The clusters follow no region and no image version: both clusters appear in `centralus`,
`westus2` and `westus3`. They are taken to be different hardware behind the same `ubuntu-24.04` label.
When the clusters are mixed, the replay's typical tolerance is 22% on the fuzzy row and 48% on the
IPC row, which is too loose to catch anything. **This is why the CPU model is part of the key.**

**2. Within one cluster, the spread can be measured.** `series-split-by-mode.jsonl` splits the
history at 780 µs on the fuzzy row, as a stand-in for the CPU model that was not recorded. At K = 4
with a window of 30:

| series | points | scaled MAD | tolerance, smallest / typical | flagged |
|---|---|---|---|---|
| fuzzy, slow mode | 86 | 4.19% | 6.5% / 9.1% | 2 of 56: +8.9%, +21.4% |
| IPC, slow mode | 86 | 3.79% | 8.7% / 17.0% | 0 of 56 |
| fuzzy, fast mode | 36 | 13.67% | 47.1% / 54.2% | 0 of 6 |
| IPC, fast mode | 36 | 6.80% | 26.0% / 26.1% | 0 of 6 |

A point is flagged when it is above the limit computed from the 30 points before it. The +8.9% point is
the first of a sustained step. From commit `c7749088` (#217) onward, the slow-mode fuzzy median moved
from 878 µs to 950 µs and stayed there. The IPC row and the image version did not move, which is
consistent with a code change. K = 4 flags that first point and K = 5 lets it through. The +21.4%
point is a lone reading: the runs after it were back at 957 µs and 918 µs. K = 4, 5 and 6 all flag it.

The fast mode's spread is much wider than the slow mode's. That suggests the fast mode is itself more
than one CPU model, which the CPU key now separates. Keyed series should give tighter tolerances than
this table shows.

**3. Short windows underestimate the spread.** `series-step-free.jsonl` is the same history with the
step split into its own series, so that every flag in it is a false alarm. At K = 4:

| window | flagged |
|---|---|
| 10 | 8 of 194 |
| 15 | 5 of 169 |
| 20 | 3 of 144 |
| 30 | **0 of 94** |

Each of those flags would have been a red `main` with no regression behind it. That is why the gate
judges only with a full window of 30. At a window of 30, K = 4 and K = 5 both flag none of the 94
points. K = 4 is chosen because it also catches the real step in table 2.

**4. What the gate can catch.** In this history, a single run is flagged when it is about 9% above
the slow-mode fuzzy median, or about 17% above the slow-mode IPC median. A smaller regression shows
in the series but does not block `main`. The step in table 2 blocked one run and then became the new
baseline, as a rolling median does. Rerun `analyse` on the keyed series once it has 30 points per
model, and revisit K and the window if the spread has changed.

**5. The 1% minimum** matters in one case: a window whose MAD is zero, such as a run of identical
page-granular RSS readings. In that window, any increase at all would block `main`. The minimum is
not the operative tolerance on any series measured so far, because the smallest 4-sigma tolerance in
the replay is 6.5%.

No idle-RSS series existed before this change, because the figure was only printed. Its tolerance
comes from the same rule, applied to its own series, once 30 points exist for an environment.

## What blocks, and when

- **A push to `main`:** a regression fails the `perf-gate` job (`--blocking`).
- **A pull request, a scheduled run or a manual run:** a regression produces a warning annotation and
  a row in the step summary. The job stays green.
- **Any event:** a run with no figure for an expected metric fails. Either the bench did not run or a
  metric was renamed, and a gate that measured nothing must not pass.
- **Not judged, on any event:** an environment with fewer than 30 points, a metric with no baseline,
  and a metric whose only baseline is from another environment. In the last case the report lists the
  fields that differ. The gate never compares across environments.

## Accepting a deliberate regression

The window is a rolling median, so it adapts to a sustained change. A regression that is larger than
the tolerance would still keep `main` red until about fifteen runs had recorded it. To accept such a
regression, add an entry to [`scripts/ci/perf-resets.json`](../../scripts/ci/perf-resets.json) in the
PR that causes it:

```json
{"metric": "slas.fuzzy_rank_top20_of_10k", "after": "2026-11-20T00:00:00Z",
 "reason": "Ranking now scores acronyms; +12% measured in #NNN."}
```

Points recorded before `after` no longer count. The metric is reported, not judged, until 30 new
points exist. At the scheduled rate that takes about eleven days for the commonest CPU model.

## Against upstream Vicinae

#310 asks that every action Compass and upstream can both perform is no slower and no larger in
Compass than in the pinned upstream release. The `upstream-compare` job in `perf.yaml` runs
`just bench-compare` ([`scripts/bench/compare.sh`](../../scripts/bench/compare.sh)) on a hosted
runner, and the `upstream-gate` job runs `perf-gate.py versus` on what it measured.

### Same machine, same session, same action

`compare.sh` verifies the v0.29.0 AppImage named in
[`baseline.json`](../../scripts/bench/baseline.json) against its SHA-256 and builds this commit in
release mode. `compare.py` then starts one headless Sway and cold-starts each engine in turn, five
times each, alternating which engine goes first. Both engines index the same corpus, run under the
same network setting and are timed by the same code. [BENCHMARKS.md](./BENCHMARKS.md) defines each
measurement. The gated actions are `GATED` in `compare.py`:

| metric | action | Compass and upstream both |
|---|---|---|
| `versus.ping` | warm IPC ping, ns | are timed by `head-to-head` from `compass-testkit`: one client, persistent sockets, AB/BA order, 1,000 samples after 100 warm-ups, once per run |
| `versus.query_keystroke` | `wtype fire` until the next changed frame, ms | are timed from screenshots |
| `versus.launch_first_frame` | exec until the first frame, ms | are timed from screenshots |
| `versus.launch_populated` | exec until the launcher stops changing, ms | are timed from screenshots |
| `versus.idle_rss` | process-tree RSS 5 s after the first frame, KiB | are read from `smaps_rollup` |

Everything else `compare.py` measures is listed in `EXCLUDED`, with the reason it is not gated. The
gate report prints that list on every run. Engine readiness is excluded because the probes differ:
Compass is polled through `compass ping`, which spawns a process each time. Search over IPC is
excluded because unmodified v0.29.0 has no `Ipc/rootQuery`, which [HEAD-TO-HEAD.md](./HEAD-TO-HEAD.md)
explains. The fuzzy scorers are excluded because they accept different match sets and so do
unequal work.

### A point, and its series

One point per action and run. Its `value` is Compass's median. Its `upstream` field holds
upstream's median from the same run, with the repository, tag, commit and AppImage SHA-256. Its
`env` adds `network` and `corpus` to the usual key, because upstream fetches currency rates at start
when it has a network, and because a different corpus is a different workload. A hosted runner
cannot run `unshare --net`, so CI's series has `network=host` for both engines. The report says so.

Points are compared only when the environment key, the network, the corpus digest and the upstream
artifact all match. Changing `baseline.json` therefore starts a new series. The artifact is
`upstream-points-<head sha>`, and it carries `compare.py`'s `report.json` as well. The gate job
fetches the series from up to 150 earlier `perf.yaml` runs on `main` with
`perf-gate.py fetch --pattern 'upstream-points-*'`.

### The tolerance

Each point gives a ratio: Compass's median over upstream's, both from the same run. An action fails
when this run's ratio is above

$$1 + \max(4 \times 1.4826 \times \text{MAD} / \text{median},\ 1\%)$$

where the median and the MAD are the ratio's, over the last **30** compatible runs on `main`. The
gate does not judge until it has 30 such runs. That is "Compass median ≤ upstream median × (1 +
tolerance)", with the tolerance taken from the measured run-to-run spread of the comparison itself.

The ratio is used rather than either engine's own series because both engines ran on the same
machine within minutes of each other. A slower runner slows both engines, so its effect largely
cancels in the ratio, and the ratio's spread is the noise that remains in the comparison. Upstream's
own series would also include the machine-to-machine spread, which would make the tolerance looser
than the comparison needs. K = 4, the window of 30 and the 1% minimum are the regression gate's,
for the reasons given [above](#the-tolerance-and-the-analysis-behind-it). The window includes
Compass's own changes. That only widens the tolerance, and the regression gate covers those
changes.

`perf-gate.py analyse` replays a `versus` series in the same way, using the ratios. Rerun it once
the hosted series has 30 runs per CPU model, before trusting the first red `main`.

### What blocks

The policy is the regression gate's. On a push to `main`, an action that is behind upstream by more
than the tolerance fails `upstream-gate`. On any other event it is a warning. A run that is missing
one of the five actions fails on any event. Until `main` has 30 compatible runs, every action is
reported and not judged.

A reset in `perf-resets.json` works for a `versus.*` metric as it does for any other metric. It
restarts that action's ratio window. It does not let Compass stay behind upstream: once 30 new runs
exist, the action is judged against a ratio of 1 again.

The first run of `compare.py` with the gate was a local one, with three runs of each engine on a
shared 12-core AMD EPYC with a load average of 7 to 14, so its figures are noisy. Compass was ahead
on `ping`, at a ratio of 0.58, and on `query_keystroke`, at 0.21. It was behind on both launch
actions, at 2.5 and 1.5, and level on `idle_rss`, at 1.004. Compass's first frame ranged from 0.5 s
to 19 s across its three runs. This is why the gate does not judge before it has a series: one run
on a busy machine cannot tell a real gap from noise. If the hosted series confirms the launch gap,
`main` turns red on those two actions as soon as the series is long enough to judge.

## The resident window on real hardware

ADR-0018 moved the resident window's idle RSS on real hardware to #127. That series must be recorded
**before** a threshold is chosen. It cannot be gathered in CI or in the containers this repository is
usually worked on, because neither has a GPU. On a real GPU session, run Compass with `compass start`,
leave the window open and idle, and then run:

```sh
python3 scripts/ci/perf-gate.py measure-rss --pid "$(pgrep -f 'compass launcher-child' | head -1)" \
  --metric window.idle_rss_kb --renderer amdgpu --profile release --runner local \
  --sha "$(git rev-parse HEAD)" --out window-rss-hardware.jsonl
```

Name the renderer after the driver that wgpu actually used, such as `amdgpu`, `i915`, `xe` or
`nvidia`. Readings from different renderers are never compared. Once there are enough readings,
`perf-gate.py analyse window-rss-hardware.jsonl` gives the series' spread, which is the evidence a
window budget needs. **No such series has been recorded yet.**
