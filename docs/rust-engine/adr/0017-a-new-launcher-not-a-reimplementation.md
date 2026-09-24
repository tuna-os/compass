# ADR-0017: Compass is a new launcher, not a reimplementation of Vicinae

**Status:** Accepted · **Date:** 2026-09-24 · Supersedes: ADR-0014 · Amends: PLAN.md §5, §8.1, the Phase 1 and Phase 5 gates

## Context

The plan was written as a port. Its safety net (Suite 0), its ledger (`PARITY.md`), its phase gates
and several of its crates all measure one thing: does the Rust engine do what the C++ engine does.
That was the right question for a migration that would one day replace Vicinae's core in place.

It is not the question the project owner is asking. Stated directly:

> Not to have byte for byte behaviour repro of vicinae. We want a performant, faster and more
> efficient launcher in the spirit of vicinae. We aren't going to upstream a replacement for it —
> this is a new project.

and, on how it should be built: use off-the-shelf Rust crates where that makes sense, and only
hand-roll what is genuinely missing from the ecosystem.

Two things in the tree do not survive that reading.

**Differential parity was the spec, and it can only ever say "different".** The first real Suite 0
run ([`SUITE0-BASELINE.md`](../SUITE0-BASELINE.md)) agrees on 99.0% of top results, and the
remaining 1% is fzf-versus-nucleo word-boundary taste — neither side is wrong. Meanwhile the
absolute quality suite (`crates/compass-core/tests/search_quality.rs`) found a real defect,
[#204](https://github.com/tuna-os/compass/issues/204): one transposed keystroke drops the app from
the results entirely. The C++ engine has the same defect, so the differential sees agreement. A
harness that measures sameness cannot find a bug both engines share, and cannot tell an improvement
from a regression.

**Some code exists only to be byte-compatible with Vicinae.** The largest case is the storage stack
ADR-0014 required: `compass-sqlcipher-sys` (hand-written FFI, the one crate that must opt out of
`unsafe_code = "forbid"`) plus a vendored C tokenizer of ~1300 lines, most of it a verbatim copy of
SQLite's own `fts5_unicode2.c`. Its whole justification is ADR-0014's first line: "Two facts about
the *existing on-disk format* constrain the dependency choice." Without the requirement to open
Vicinae's files in place, neither fact constrains anything — `rusqlite` bundles SQLite and
SQLCipher, and SQLite has shipped a `trigram` tokenizer since 3.34 (`remove_diacritics` since 3.45).

## Decisions

**1. Quality is asserted absolutely; the C++ engine is a tripwire, not the spec.** A feature area is
covered when it has tests that state what *good* looks like — the search-quality suite, the paint
tier, the end-to-end engine tests — and would fail on a regression whatever the C++ engine does.
Suite 0 keeps running: a sudden divergence is a cheap signal that *something* changed. A divergence
that makes Compass better is declared in `PARITY.md` and kept, not fixed toward C++.

The CI gate on Suite 0 is left exactly as it is (top result, queries of four or more characters)
until the project owner decides otherwise. Loosening a merge gate is theirs to call, and this ADR
does not call it.

**2. Crates first; hand-rolled code states why.** Before writing a subsystem, name the maintained
crate that does it, or say in the crate's top-level doc comment why none fits. "The C++ engine did
it this way" is not a reason. Existing hand-rolled crates were reviewed against this (see Consequences);
the ones that stay have a reason other than parity.

**3. User data is imported, not shared.** Compass owns its storage formats. Continuity for someone
coming from Vicinae — clipboard history, extension storage, OAuth tokens, config — is a one-shot
importer at first run, not a constraint on how Compass stores anything. An importer only needs to
read the *content* tables of a Vicinae database, never its FTS index, so it does not need the
vendored tokenizer either. (ADR-0014 measured that a plain `SELECT` on the FTS table fails without
it; the content tables are ordinary tables.)

**4. ADR-0014 is superseded.** New stores use `rusqlite` — `bundled-sqlcipher` where the data is
secret (OAuth tokens, extension storage, clipboard), plain `bundled` where it is not (the file
index) — and SQLite's built-in `trigram` tokenizer. `compass-sqlcipher-sys` and
`vendor/fuzzy-trigram` leave the Rust engine's dependency graph, and so does `vendor/spellfix`: the
file index's typo fallback moves into Rust over an off-the-shelf edit-distance crate. Encryption at rest is kept: it is
a property users rely on, not a compatibility detail.

**5. Compatibility identifiers from ADR-0012 are unchanged.** The `vicinae` binary, socket, config
path and `@vicinae/api` package stay until cutover supplies aliases. Those protect users' scripts
and extensions — a real cost to break — which is a different thing from reproducing internals.

## Consequences

- **The phase gates change wording, not rigour.** Phase 1's "Suite 0 parity green on the 500-entry
  corpus" becomes "the search-quality suite green on the real corpus, and Suite 0 green at its
  current gate". Phase 5's "parity ledger ≥ 95% green" becomes "every feature area in the ledger
  has absolute tests". Both are stricter in the way that matters: they fail on shared bugs.
- **`PARITY.md` becomes a feature ledger.** The "C++ ✓" column stays as provenance; the column that
  gates is "tested ✓".
- **Storage migration is real work** — four crates call `compass-sqlcipher-sys` (`compass-db`,
  `compass-local-storage`, `compass-oauth-store`, `compass-worker-host`). It is sequenced after
  #204 in PLAN.md §12 and lands behind the existing storage tests, which must pass unchanged
  except where they asserted Vicinae's byte format.
- **Reviewed and kept** under decision 2: `compass-search` (nucleo underneath; the hand-rolled part
  is ranking policy, which is product, not plumbing), `compass-xdg` (fixes six upstream parser bugs
  rather than reproducing them; `freedesktop-desktop-entry` is worth re-evaluating, lowest
  priority), `compass-notify` and `compass-media` (thin over `zbus`; no crate does the whole job).
- **Upstream remains credited** as the source project and a behavioural reference. The six
  desktop-entry bugs in [`upstream-bug-report.md`](../upstream-bug-report.md) are still worth
  reporting as a courtesy; filing is the owner's call.

## What would change this

A decision to upstream the Rust engine into Vicinae after all. That would restore byte-level
compatibility as a requirement, and ADR-0014's reasoning would apply again as written.
