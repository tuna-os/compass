# ADR-0006: Reconstruct fzf's coherence signal over nucleo's match indices

**Status:** Accepted · **Date:** 2026-09-12 · Relates to: PARITY.md, PLAN.md §8.3

## Context

Porting `src/lib/fuzzy` onto `nucleo-matcher` reproduced the C++ behaviour closely — every
normalized `score`/`quality` assertion in the C++ suite ports verbatim — with one significant gap.

The C++ matcher computes a `coherent` flag inside its backtracking pass, from the DP matrix's
per-position boundary bonuses, and forces `quality = 0` for incoherent matches. nucleo exposes
neither the matrix nor an equivalent. Consequence: C++ rejects `"time"` against
*"Play this game on Steam"* and *"Start Input Method"*; we accepted them at quality 69 and 73
against a gate of 60.

That is a user-visible false-positive source — a launcher that surfaces nonsense for short queries
is a launcher people stop trusting — and it blocked deleting `src/lib/fuzzy`.

Three options: layer a classifier over nucleo's indices; raise `MIN_QUALITY` globally; accept the
looser matching as a product decision.

## Decision

**Layer a coherence classifier over nucleo's match indices.**

## Why not the alternatives

- **Raising `MIN_QUALITY`** trades false positives for false negatives globally. The failing cases
  score 69–73 while legitimate matches live in the same band, so any threshold that rejects
  *"time"* against *"Play this game on Steam"* also rejects real matches. It treats a
  shape problem as a magnitude problem.
- **Accepting looser matching** silently changes behaviour every existing user relies on, in the
  direction of "the launcher got worse". Not a decision to make by default.

## What this costs, honestly

We are approximating an algorithm we cannot call, from less information than it had: nucleo gives us
matched indices, the haystack and the needle — not the DP matrix. Exact reproduction is not
guaranteed, and the classifier is a heuristic with tuned parameters that will need re-tuning.

The mitigation is method: tune against the whole table of C++ expectations at once, never a single
case; keep every previously-passing test passing; and where a case genuinely cannot be reproduced
from indices alone, leave it recorded as a divergence rather than faking closure. A smaller honest
gap beats a fake one.

## Consequences

- The tuned parameters and the reasoning behind them are documented next to the classifier, so the
  next person re-tunes rather than reverse-engineers.
- Suite 0 differential testing against the C++ engine is how this stays honest over time; the
  classifier is exactly the kind of heuristic that drifts.
- `src/lib/fuzzy` cannot be deleted until the remaining gap is recorded and accepted.
