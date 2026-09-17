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

# The real gates, kept identical to launcher.sh. If they drift, this file is
# testing something the VM tier does not run.
GATE = [
    "--min-percent", "3",
    "--expect-box", "300", "140", "980", "800",
    "--ignore-box", "0", "0", "1279", "139",
]

# ADR-0015's gate: the engine hides the window, and the screen must go back to
# looking like the bare desktop. It is the mirror of GATE and needs its own
# controls for the same reason -- `--max-percent` passing is what "nothing
# changed" looks like, so a gate that could never fail would look identical to
# a window that reliably went away.
AWAY_GATE = [
    "--max-percent", "3",
    "--ignore-box", "0", "0", "1279", "139",
    "--ignore-box", "0", "700", "1279", "799",
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


def dash_only(x: int, y: int) -> tuple[int, int, int]:
    """A change confined to the bottom strip, where the shell's dash lives.

    The real thing the second ignore box exists for: the dash's backdrop is
    redrawn when a process starts, which failed the engine-draws-nothing gate
    at 2.04% before it was excluded.
    """
    if 520 <= x <= 760 and 716 <= y <= 799:
        return WINDOW
    return desktop(x, y)


def wrong_place(x: int, y: int) -> tuple[int, int, int]:
    """A big window, but down the left edge — outside the expected box."""
    if 10 <= x <= 290 and 300 <= y <= 790:
        return WINDOW
    return desktop(x, y)


AWAY_CASES = [
    # (name, after-painter, must-pass, why this case is here)
    ("the window went away", desktop, True,
     "the ordinary success: hiding puts the screen back to the desktop"),
    ("went away, but the clock ticked", with_clock(desktop), True,
     "the same top-bar noise that broke the open gate, on the hide path"),
    ("the window is still there", with_launcher, False,
     "THE ONE THAT MATTERS: proves the gate can fail at all, rather than "
     "passing on every frame because --max-percent is satisfied by anything "
     "that did not change"),
    ("the window moved but stayed", wrong_place, False,
     "a window that left the expected box is still a window on screen"),
]

# Step 0d: starting the engine must put nothing on screen. A tighter bound than
# the others because there is nothing legitimate for it to draw at all -- which
# is also what makes it the gate that actually exercises the dash exclusion.
#
# The dash strip is 240x84 = 20160 px, about 2.4% of the compared area. Under
# the away gate's 3% that passes with or without the second ignore box, so a
# control placed there proves nothing -- a control confirmed exactly that. At
# 1% it does not.
ENGINE_GATE = [
    "--max-percent", "1",
    "--ignore-box", "0", "0", "1279", "139",
    "--ignore-box", "0", "700", "1279", "799",
]

ENGINE_CASES = [
    ("nothing changed", desktop, True,
     "the ordinary success: a headless engine draws nothing"),
    ("only the dash changed", dash_only, True,
     "THE ONE THAT MATTERS: the shell redraws the dash backdrop when a process "
     "starts -- measured at 2.04%, which fails this gate's 1% unless the second "
     "ignore box is in effect"),
    ("a dialog in the middle of the screen", with_launcher, False,
     "the hotkey permission dialog coming back must still fail, or excluding "
     "two strips has gutted the gate"),
]

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

    # EVERY invocation, not the first: launcher.sh now runs framediff three
    # times -- the open gate, the went-away gate and the came-back gate -- and
    # matching only the first would let the other two drift unguarded.
    #
    # The actual invocations, not mentions: the word framediff.py also appears
    # in the comments above them, and slicing from one of those produced an
    # error message quoting prose instead of a command.
    lines = launcher.splitlines()
    invocations = []
    for start, line in enumerate(lines):
        if not (line.strip().startswith("python3") and "framediff.py" in line):
            continue
        end = start
        while end < len(lines) - 1 and lines[end].rstrip().endswith("\\"):
            end += 1
        invocations.append("\n".join(lines[start : end + 1]))

    if not invocations:
        raise SystemExit(
            "launcher.sh no longer invokes framediff.py; these controls guard nothing."
        )

    # Keyed on the frame each gate compares against, not on its flags alone.
    # The open gate and the came-back gate use identical flags, so a flags-only
    # check is satisfied by either of them -- a control confirmed that deleting
    # the came-back invocation left this guard green.
    wanted = {
        "the open gate": ("launcher-01-open.png", "--min-percent 3",
                          "--expect-box 300 140 980 800", "--ignore-box 0 0 1279 139"),
        "the went-away gate": ("launcher-04-hidden.png", "--max-percent 3",
                               "--ignore-box 0 0 1279 139",
                               "--ignore-box 0 700 1279 799"),
        "the came-back gate": ("launcher-05-summoned.png", "--min-percent 3",
                               "--expect-box 300 140 980 800", "--ignore-box 0 0 1279 139"),
        # Starting the engine must be invisible, now that it runs --no-hotkey.
        # A tighter bound than the others on purpose: there is nothing legitimate
        # for it to draw at all, so the only slack is the shell's own furniture.
        "the engine-draws-nothing gate": ("launcher-00a-bare-desktop.png",
                                          "--max-percent 1",
                                          "--ignore-box 0 0 1279 139",
                                          "--ignore-box 0 700 1279 799"),
    }

    for what, flags in wanted.items():
        if not any(all(flag in inv for flag in flags) for inv in invocations):
            raise SystemExit(
                f"framediff self-test is testing {what}, which launcher.sh does not run.\n"
                f"  expected all of: {list(flags)}\n"
                "  launcher.sh invocations were:\n"
                + "\n\n".join(
                    "\n".join(f"    {line}" for line in inv.strip().splitlines())
                    for inv in invocations
                )
                + "\n  Update the gates in this file and re-run the controls."
            )


def main() -> int:
    assert_gate_matches_launcher_sh()
    failures = []
    with tempfile.TemporaryDirectory() as tmp:
        tmp = Path(tmp)
        before = tmp / "before.png"
        write_png(before, desktop)

        cases = [("appeared", GATE, case) for case in CASES]
        cases += [("went away", AWAY_GATE, case) for case in AWAY_CASES]
        cases += [("engine draws nothing", ENGINE_GATE, case) for case in ENGINE_CASES]

        for gate_name, gate, (name, painter, must_pass, why) in cases:
            name = f"[{gate_name}] {name}"
            after = tmp / "after.png"
            write_png(after, painter)
            result = subprocess.run(
                [sys.executable, str(FRAMEDIFF), str(before), str(after), *gate],
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
    print(
        f"framediff self-test: all {len(CASES) + len(AWAY_CASES) + len(ENGINE_CASES)} "
        "controls behaved as required"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
