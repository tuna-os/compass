#!/usr/bin/env python3
"""Controls for framediff.py's launcher gate.

WHY THIS FILE EXISTS

`scripts/vmtest/launcher.sh` asserts that opening the launcher changes at least
3% of the screen inside a box. That assertion failed on a run where the
launcher had painted perfectly, because `--expect-box` requires the changed
region to lie *within* the box, which silently also requires that nothing else
on screen changed. GNOME's top bar carries a clock. Two runs say it exactly:

    passing   84077 changed   box x 335..942  y 152..796
    failing   84172 changed   box x 335..942  y  10..796

Ninety-five extra pixels in the top bar, the launcher's own footprint identical
to the pixel, and a red build.

The fix was `--ignore-box`, which excludes the shell's furniture. An
exclusion is exactly the kind of change that can quietly gut an assertion, so
it is not enough that the real gate went green: the gate has to be shown still
failing for every reason it existed to catch. That is what this checks, on
synthetic frames, in CI, with no VM.

Run it directly; it prints each case and exits non-zero if any behaves wrongly.
"""

from __future__ import annotations

import struct
import subprocess
import sys
import tempfile
import zlib
from pathlib import Path

HERE = Path(__file__).resolve().parent
FRAMEDIFF = HERE / "framediff.py"

W, H = 1280, 800

# The real gate, kept identical to launcher.sh. If they drift, this file is
# testing something the VM tier does not run.
GATE = [
    "--min-percent", "3",
    "--expect-box", "300", "140", "980", "800",
    "--ignore-box", "0", "0", "1279", "139",
]

DESKTOP = (20, 20, 30)
WINDOW = (200, 200, 210)
CLOCK = (255, 255, 255)


def write_png(path: Path, painter) -> None:
    """Minimal RGB8 PNG writer, filter type 0 throughout."""
    rows = bytearray()
    for y in range(H):
        rows.append(0)
        for x in range(W):
            rows += bytes(painter(x, y))

    def chunk(kind: bytes, body: bytes) -> bytes:
        return (
            struct.pack(">I", len(body))
            + kind
            + body
            + struct.pack(">I", zlib.crc32(kind + body) & 0xFFFFFFFF)
        )

    path.write_bytes(
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", struct.pack(">IIBBBBB", W, H, 8, 2, 0, 0, 0))
        + chunk(b"IDAT", zlib.compress(bytes(rows)))
        + chunk(b"IEND", b"")
    )


def desktop(x: int, y: int) -> tuple[int, int, int]:
    return DESKTOP


def with_launcher(x: int, y: int) -> tuple[int, int, int]:
    """The launcher exactly where two calibration runs measured it."""
    if 335 <= x <= 942 and 152 <= y <= 796:
        return WINDOW
    return desktop(x, y)


def with_clock(painter):
    """Overlay a changed clock in the top bar, as the failing run had."""

    def paint(x: int, y: int) -> tuple[int, int, int]:
        if 600 <= x <= 680 and 8 <= y <= 20:
            return CLOCK
        return painter(x, y)

    return paint


def wrong_place(x: int, y: int) -> tuple[int, int, int]:
    """A big window, but down the left edge — outside the expected box."""
    if 10 <= x <= 290 and 300 <= y <= 790:
        return WINDOW
    return desktop(x, y)


CASES = [
    # (name, after-painter, must-pass, why this case is here)
    ("launcher only", with_launcher, True,
     "the ordinary success; if this fails the gate is broken outright"),
    ("launcher plus a clock tick", with_clock(with_launcher), True,
     "the exact shape of the red run this fix is for"),
    ("clock tick only, no launcher", with_clock(desktop), False,
     "THE ONE THAT MATTERS: proves --ignore-box did not turn top-bar noise "
     "into a passing 'a window appeared'"),
    ("window outside the expected box", wrong_place, False,
     "proves containment is still enforced where it is not ignored"),
    ("nothing changed", desktop, False,
     "proves the gate still catches a launcher that never drew"),
]


def assert_gate_matches_launcher_sh() -> None:
    """The gate tested here must be the gate the VM tier runs.

    Without this, launcher.sh could be retuned and these controls would go on
    passing against the old flags -- a test proving something nothing runs.
    It is the same failure as two parsers disagreeing about one corpus.
    """
    launcher = (HERE / "launcher.sh").read_text()

    # The actual invocation, not the first mention: the word framediff.py also
    # appears in the comment above it, and slicing from that produced an error
    # message quoting prose instead of the command.
    lines = launcher.splitlines()
    start = next(
        (i for i, line in enumerate(lines)
         if line.strip().startswith("python3") and "framediff.py" in line),
        None,
    )
    if start is None:
        raise SystemExit(
            "launcher.sh no longer invokes framediff.py; these controls guard nothing."
        )
    end = start
    while end < len(lines) - 1 and lines[end].rstrip().endswith("\\"):
        end += 1
    invocation = "\n".join(lines[start : end + 1])
    missing = [
        flag
        for flag in ("--min-percent 3", "--expect-box 300 140 980 800",
                     "--ignore-box 0 0 1279 139")
        if flag not in invocation
    ]
    if missing:
        raise SystemExit(
            "framediff self-test is testing a gate launcher.sh does not run.\n"
            f"  not found in launcher.sh: {missing}\n"
            "  launcher.sh invocation was:\n"
            + "\n".join(f"    {line}" for line in invocation.strip().splitlines())
            + "\n  Update GATE in this file and re-run the controls."
        )


def main() -> int:
    assert_gate_matches_launcher_sh()
    failures = []
    with tempfile.TemporaryDirectory() as tmp:
        tmp = Path(tmp)
        before = tmp / "before.png"
        write_png(before, desktop)

        for name, painter, must_pass, why in CASES:
            after = tmp / "after.png"
            write_png(after, painter)
            result = subprocess.run(
                [sys.executable, str(FRAMEDIFF), str(before), str(after), *GATE],
                capture_output=True,
                text=True,
            )
            passed = result.returncode == 0
            verdict = "ok  " if passed == must_pass else "WRONG"
            want = "pass" if must_pass else "fail"
            print(f"{verdict}  {name}: must {want}, exit {result.returncode}")
            print(f"        {why}")
            for line in (result.stdout + result.stderr).strip().splitlines():
                print(f"        | {line}")
            print()
            if passed != must_pass:
                failures.append(name)

    if failures:
        print(f"framediff self-test FAILED: {', '.join(failures)}", file=sys.stderr)
        return 1
    print(f"framediff self-test: all {len(CASES)} controls behaved as required")
    return 0


if __name__ == "__main__":
    sys.exit(main())
