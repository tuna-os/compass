#!/usr/bin/env python3
"""Count the parity ledger's cells, by column, so the Phase 5 figure in
PLAN.md is measured rather than estimated.

Run: python3 scripts/ci/parity-score.py [docs/rust-engine/PARITY.md]
     python3 scripts/ci/parity-score.py --selftest

The `C++ ✓` and `C++ deleted ✓` columns describe the C++ tree rather than
this port (the latter is ✅ on every row since ADR-0021 removed that tree), so
they are printed but the Phase 5 figure is taken over `Rust ✓` and
`parity test ✓` alone.

It also reports what the remaining work *is*, by reading the "Still C++-only:"
sentences the notes already carry. A percentage says how far there is to go and
nothing about what the going consists of, and at this stage those have very
different answers.
"""

import collections
import sys

MARKERS = ("✅", "❌", "🟡", "⏳", "n/a", "never")
COLUMNS = ("C++ ✓", "Rust ✓", "parity test ✓", "C++ deleted ✓")


# What the notes say is left, by the kind of work it is.
#
# A percentage says how far there is to go and nothing about what the going
# consists of, and those are different questions with different answers: a
# ledger at 44% whose remainder is transcription is a week of typing, and one
# whose remainder is compositor integration is not. This reads the "Still
# C++-only:" sentences the notes already carry and counts what they name, so
# the answer stays true as rows land rather than being an opinion someone wrote
# down once.
KINDS = {
    "view": ("view", "qml", "widget", "grid widget", "specimen", "swatch", "pane"),
    "backend": ("dbus", "mpris", "provider", "wayland plumbing", "bus connection", "registry"),
    "network": ("http", "fetch", "install-from-zip"),
    "process": ("launch", "spawn", "starts a process", "terminal"),
    "storage": ("sqlite", "database", "migration", "store behind"),
}


def classify(gap: str) -> set:
    """The kinds a 'Still C++-only:' sentence names."""
    lowered = " ".join(gap.split()).lower()
    return {kind for kind, words in KINDS.items() if any(w in lowered for w in words)}


def selftest() -> int:
    """Check the classifier against sentences taken from the ledger.

    A bucket that silently swallows everything, or one that matches nothing,
    both produce a plausible-looking report -- so the cases below include one
    that must land in *no* bucket and one that must land in two.
    """
    cases = [
        ("the QML views, the detail pane, the drag payload", {"view"}),
        ("the MPRIS provider, the audio provider, and the Now Playing view",
         {"backend", "view"}),
        ("the HTTP calls, the install-from-zip path, and the QML views",
         {"network", "view"}),
        ("everything that starts a process (launch, the file browser)",
         {"process"}),
        ("the migration from the old OmniDatabase, and resolveApp", {"storage"}),
        ("resolveApp", set()),
    ]
    failures = 0
    for sentence, expected in cases:
        got = classify(sentence)
        if got != expected:
            print(f"FAIL {sentence!r}\n  expected {sorted(expected)}, got {sorted(got)}")
            failures += 1
    print(f"classifier selftest: {len(cases) - failures}/{len(cases)} passed")
    return 1 if failures else 0


def main() -> int:
    if len(sys.argv) > 1 and sys.argv[1] == "--selftest":
        return selftest()
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

    print()
    report_remaining_kinds(path)
    return 0




def report_remaining_kinds(path: str) -> None:
    import re

    text = open(path, encoding="utf-8").read()
    gaps = re.findall(r"Still C\+\+-only:(.{0,400}?)\.\n", text, re.S)
    if not gaps:
        print("remaining work by kind: no 'Still C++-only:' notes found")
        return

    counts = collections.Counter()
    unclassified = 0
    for gap in gaps:
        matched = classify(gap)
        if matched:
            counts.update(matched)
        else:
            unclassified += 1

    print(f"remaining work named by {len(gaps)} notes, by kind:")
    for kind, count in counts.most_common():
        print(f"  {kind:10} {count}")
    if unclassified:
        print(f"  {'other':10} {unclassified}")


if __name__ == "__main__":
    raise SystemExit(main())
