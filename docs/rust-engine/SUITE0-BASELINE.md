# Suite 0 baseline — the first measured differential

Recorded before gated, per [ADR-0010](adr/0010-corral-vm-tier.md). This is the measurement
[#12](https://github.com/tuna-os/compass/issues/12) and [#4](https://github.com/tuna-os/compass/issues/4)'s
first exit gate have been waiting for: what the two engines actually do when asked the same
questions over the same corpus, before anyone picks a threshold.

**Source:** `parity` job, run [35959449627](https://github.com/tuna-os/compass/actions/runs/35959449627),
head `45dbbe0`. Both engines in one Bluefin container, 757 staged entries via `XDG_DATA_DIRS`,
1817 queries derived from the corpus plus ten generic terms. C++ narrowed with
`--provider applications`. Per-query report kept as `parity-report-45dbbe0e…` for 90 days.

## Headline: top-result agreement is 99.0%

| | count |
|---|---|
| Same top hit | 1541 |
| **Different top hit** | **15** |
| Both empty | 261 |

1541 of the 1556 queries that ranked anything agree on the first result — the one a person
actually launches. PLAN §8.1 gates **top-result parity**, and this is that number.

## Whole-ranking agreement, by query length

The disagreements are not spread evenly. They are almost entirely on very short queries.

| query length | queries | identical | known divergence | regressions | agree |
|---|---|---|---|---|---|
| 1 | 40 | 5 | 8 | 27 | 32.5% |
| 2 | 209 | 40 | 13 | 156 | 25.4% |
| 3 | 400 | 209 | 34 | 157 | 60.8% |
| 4 | 467 | 393 | 43 | 31 | **93.4%** |
| 5–9 | 268 | 259 | 6 | 3 | **98.9%** |
| 10+ | 433 | 428 | 3 | 2 | **99.5%** |

Overall: 1334 identical, 107 known divergence (same ranking, different score scale), 376 regressions.

**Two-thirds of every disagreement lives in queries of three characters or fewer**, where a fuzzy
matcher returns dozens of weak matches and the tail ordering is close to arbitrary on both sides.

## The shape of the 376

| shape | count |
|---|---|
| C++ returns items Rust does not | 123 |
| Rust returns items C++ does not | 116 |
| Each returns items the other lacks | 68 |
| Same set, different order | 69 |

Roughly symmetric, which argues against one engine being broadly wrong. The items involved are
low-scoring tail matches: `org.gnome.Nautilus` appears as a C++-only hit on 34 queries, all of
them one or two characters (`C`, `CO`, `Cr`, `D`, `E`), at scores in the 45–59 band.

## The 15 differing top results

| query | C++ top | Rust top |
|---|---|---|
| `Abo` | AlgoBox | Alexandria Book Collection Manager |
| `App`, `App `, `app` | AusweisApp | APCUPSD Monitor |
| `Dev` | Development | Devhelp |
| `Deve` | Development | Devhelp |
| `Development` | Development | Accerciser |
| `Exe` | With TryExec | External Crypto Bone Administration |
| `Ge` | FlightGear | Dave Gnukem |
| `IB` | 0 A.D. | Coco Coq in Grostesteing's base |
| `Ke` | AutoKey | Not On KDE |
| `Mob` | PyMcapostbatch | Bookworm |
| `Mu`, `Mul` | aMule | Alsa Modular Synth |
| `Wr` | CellWriter | Blob Wars : Metal Blob Solid |

There is a signature here rather than fifteen unrelated cases. The C++ winners are matches **at a
word or camelCase boundary inside the string** — a**Mule**, Auto**Key**, Cell**Writer**,
Ausweis**App**, Flight**Gear** — which is what `fzf`'s bonus structure rewards. Rust's `nucleo`
scores those boundaries differently and prefers a match that starts earlier. Only one of the
fifteen (`Dev`) is a tie at equal score; the rest are genuine scoring differences.

`Development` is worth separating from the other fourteen: both engines have an item literally
named "Development" and only C++ ranks it first, which looks less like boundary weighting and
more like an exact-title bonus one side applies and the other does not.

## What a gate could say, once someone picks

Nothing here is gated yet. Three candidates, cheapest first:

1. **Top-result parity on queries of 4+ characters.** 3 regressions in 268 for 5–9, 2 in 433 for
   10+, 31 in 467 for 4. Almost free today, and it gates the thing a user notices.
2. **Top-result parity overall**, with the 15 above as a declared allowlist that must shrink.
   Honest about where we are; forces each case to be understood rather than tolerated in bulk.
3. **Whole-ranking parity on 4+ characters.** 93.4% today, so it needs work first — the 31 are
   the list to work through.

Short queries should stay ungated until the boundary-bonus difference above is either reconciled
or declared in [`PARITY.md`](PARITY.md). Gating a 25%-agreement bucket produces a red job nobody
reads, which is the failure mode this whole suite exists to avoid.
