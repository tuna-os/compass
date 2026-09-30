#!/usr/bin/env python3
"""Judge a Suite 1 report against the ratchet, and summarise it.

    scripts/suite1/check.py <report.json> [--expected scripts/suite1/expected.json]
                            [--write-expected]

The gate (PLAN §6) is every command passing. The corpus does not, yet, and
the reasons are recorded per command in expected.json: which ones pass, and
for each that does not, the verdict it gets and why. This fails when a command expected to pass does not: a regression.

It reports, without failing, a command that now passes where it was
expected to fail (tighten the ledger with `--write-expected`), and one whose
failing verdict changed. The second is a warning rather than an error
because the ledger was recorded on one machine and CI runs on another: a
runner with a system bus fails a D-Bus extension differently from a
container without one, and neither is the host's doing.

An entry may carry per-environment overrides, for a command whose result
is decided by what the environment has rather than by the host: gnome-dnd
reads GSettings schemas the Flatpak's runtime ships and the runner does not.
`--environment flatpak` judges against `entry["flatpak"]` where there is
one, and against the entry itself where there is not.

The summary goes to stdout and, in GitHub Actions, to the job summary.
"""

import argparse
import collections
import json
import os
import pathlib
import sys

HERE = pathlib.Path(__file__).resolve().parent


def key(outcome):
    return f"{outcome['extension']}:{outcome['command']}"


def first_line(text):
    return (text or "").strip().splitlines()[0][:160] if text else ""


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("report", type=pathlib.Path)
    parser.add_argument("--expected", type=pathlib.Path, default=HERE / "expected.json")
    parser.add_argument("--write-expected", action="store_true")
    parser.add_argument("--environment", choices=["host", "flatpak"], default="host",
                        help="which of an entry's per-environment verdicts applies")
    args = parser.parse_args()

    report = json.loads(args.report.read_text())
    outcomes = {key(o): o for o in report["outcomes"]}

    if args.write_expected:
        # This run is one environment's; the other's overrides are kept.
        previous = (json.loads(args.expected.read_text())
                    if args.expected.exists() else {})
        ledger = {
            k: {"verdict": o["verdict"], **({"why": first_line(o.get("detail"))}
                                            if not o["pass"] else {}),
                **{env: previous[k][env] for env in ("flatpak",)
                   if env in previous.get(k, {})}}
            for k, o in sorted(outcomes.items())
        }
        args.expected.write_text(json.dumps(ledger, indent=2, ensure_ascii=False) + "\n")
        print(f"wrote {len(ledger)} entries to {args.expected}")
        return 0

    expected = {
        k: want.get(args.environment, want)
        for k, want in json.loads(args.expected.read_text()).items()
    }
    regressions, moved, improved, missing = [], [], [], []
    for k, want in expected.items():
        got = outcomes.get(k)
        if got is None:
            missing.append(k)
            continue
        want_pass = want["verdict"] in ("rendered", "ran")
        if want_pass and not got["pass"]:
            regressions.append((k, want["verdict"], got["verdict"], first_line(got.get("detail"))))
        elif not want_pass and got["pass"]:
            improved.append((k, want["verdict"], got["verdict"]))
        elif not want_pass and got["verdict"] != want["verdict"]:
            moved.append((k, want["verdict"], got["verdict"], first_line(got.get("detail"))))

    counts = collections.Counter(o["verdict"] for o in outcomes.values())
    lines = [
        "## Suite 1: real extensions, first frame",
        "",
        f"**{report['passed']} of {report['total']} passed** "
        f"({', '.join(f'{v} {n}' for v, n in counts.most_common())})",
        "",
        "| | command | verdict | detail |",
        "|---|---|---|---|",
    ]
    for k, o in sorted(outcomes.items()):
        mark = "✅" if o["pass"] else "❌"
        lines.append(f"| {mark} | `{k}` | {o['verdict']} | {first_line(o.get('detail'))} |")
    for title, rows in (
        ("Regressions (expected to pass)", regressions),
        ("Failing differently from the ledger", moved),
        ("Now passing: tighten the ledger", improved),
    ):
        if rows:
            lines += ["", f"### {title}", ""]
            lines += [f"- `{row[0]}`: {' → '.join(row[1:3])} {row[3] if len(row) > 3 else ''}"
                      for row in rows]
    if missing:
        lines += ["", "### Not run", ""] + [f"- `{k}`" for k in missing]
    summary = "\n".join(lines) + "\n"
    print(summary)
    if os.environ.get("GITHUB_STEP_SUMMARY"):
        with open(os.environ["GITHUB_STEP_SUMMARY"], "a", encoding="utf-8") as out:
            out.write(summary)

    failed = bool(regressions)
    for row in regressions:
        print(f"::error::Suite 1 regression: {row[0]} was {row[1]}, is {row[2]}: {row[3]}")
    for row in moved:
        print(f"::warning::Suite 1 ledger out of date: {row[0]} was {row[1]}, is {row[2]}: {row[3]}")
    for k in missing:
        print(f"::warning::Suite 1: {k} is in the ledger and was not run")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
