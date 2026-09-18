#!/usr/bin/env python3
"""Count the parity ledger's cells, by column, so the Phase 5 figure in
PLAN.md is measured rather than estimated.

Run: python3 scripts/ci/parity-score.py [docs/rust-engine/PARITY.md]

The `C++ deleted` column cannot go green before Phase 8 by the ledger's own
rule, and the `C++ ✓` column describes the C++ tree rather than this port, so
the Phase 5 figure is taken over `Rust ✓` and `parity test ✓` alone.
"""

import collections
import sys

MARKERS = ("✅", "❌", "🟡", "⏳", "n/a", "never")
COLUMNS = ("C++ ✓", "Rust ✓", "parity test ✓", "C++ deleted ✓")


def main() -> int:
    path = sys.argv[1] if len(sys.argv) > 1 else "docs/rust-engine/PARITY.md"
    per_column = {name: collections.Counter() for name in COLUMNS}

    for line in open(path, encoding="utf-8"):
        if not line.startswith("|"):
            continue
        cells = [cell.strip() for cell in line.strip().strip("|").split("|")]
        if len(cells) < 7:
            continue
        for name, cell in zip(COLUMNS, cells[-4:]):
            if cell in MARKERS:
                per_column[name][cell] += 1

    for name in COLUMNS:
        counts = per_column[name]
        countable = sum(counts[m] for m in ("✅", "❌", "🟡", "⏳"))
        print(f"{name:16} {dict(counts)}  countable={countable}")

    phase5 = collections.Counter()
    for name in ("Rust ✓", "parity test ✓"):
        phase5 += per_column[name]
    countable = sum(phase5[m] for m in ("✅", "❌", "🟡", "⏳"))
    print()
    print(f"Phase 5 (Rust ✓ + parity test ✓): {dict(phase5)}")
    print(f"  {phase5['✅']} green of {countable} countable = {100 * phase5['✅'] / countable:.0f}%")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
