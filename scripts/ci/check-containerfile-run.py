#!/usr/bin/env python3
"""Parse the shell inside every Containerfile ``RUN``.

The VM tier's images are built from these, and a shell mistake in one is a
25-minute round trip to discover: the image build is the first thing the tier
does and the last thing anything else can tell you about. Nothing else in CI
looks at them -- shellcheck is pointed at ``*.sh``, and a Containerfile is not
one.

This does not run the commands, and it cannot catch what they mean. It catches
the class that is pure syntax: an unbalanced quote, a stray continuation, a
heredoc that never closes. ``ARG``/``ENV`` defaults declared in the same file
are substituted first, so an unexpanded ``"$FOO"`` does not read as an error.
"""

from __future__ import annotations

import pathlib
import re
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parents[2]
PATTERNS = ("packaging/vmtest/Containerfile*", "packaging/flatpak/Containerfile*")


def declared_defaults(text: str) -> dict[str, str]:
    """``ARG NAME=value`` and ``ENV NAME=value`` pairs, for substitution."""
    found = {}
    for line in text.splitlines():
        match = re.match(r"^(?:ARG|ENV)\s+([A-Za-z_][A-Za-z0-9_]*)=(.*)$", line.strip())
        if match:
            found[match.group(1)] = match.group(2).strip().strip('"')
    return found


def run_bodies(text: str) -> list[str]:
    """Every ``RUN`` instruction's body, continuations joined."""
    bodies: list[str] = []
    buffer: str | None = None
    for line in text.splitlines():
        if buffer is not None:
            buffer += "\n" + line
            if not line.rstrip().endswith("\\"):
                bodies.append(buffer)
                buffer = None
        elif line.startswith("RUN "):
            buffer = line[len("RUN ") :]
            if not line.rstrip().endswith("\\"):
                bodies.append(buffer)
                buffer = None
    if buffer is not None:
        bodies.append(buffer)
    return bodies


def main() -> int:
    files = sorted(
        {path for pattern in PATTERNS for path in ROOT.glob(pattern) if path.is_file()}
    )
    if not files:
        print("::error::no Containerfiles found — the paths have moved")
        return 1

    failures = 0
    checked = 0
    for path in files:
        text = path.read_text(encoding="utf-8")
        defaults = declared_defaults(text)
        for index, body in enumerate(run_bodies(text), start=1):
            for name, value in defaults.items():
                body = body.replace(f'"${name}"', value).replace(f"${{{name}}}", value)
            checked += 1
            done = subprocess.run(
                ["bash", "-n"], input=body, text=True, capture_output=True, check=False
            )
            # Stderr, not just the exit status. `bash -n` exits 0 on an
            # unterminated heredoc and only WARNS ("here-document at line 1
            # delimited by end-of-file"), so a control that truncated one came
            # back silent and this docstring's claim to catch it was false.
            # Anything bash has to say about these bodies is worth failing on;
            # they are clean today, so there is no noise to tolerate.
            if done.returncode != 0 or done.stderr.strip():
                failures += 1
                detail = done.stderr.strip() or f"exit {done.returncode}"
                print(f"::error file={path.relative_to(ROOT)}::RUN #{index}: {detail}")
        print(f"  {path.relative_to(ROOT)}: {len(run_bodies(text))} RUN instruction(s)")

    print(f"{checked} RUN body/bodies parsed across {len(files)} file(s)")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
