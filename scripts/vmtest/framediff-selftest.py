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
    #
    # The containment box is `"${expect_box[@]}"` rather than four numbers.
    # It used to be the literal `--expect-box 300 140 980 800`, taken from a
    # 640x480 window, and when the card was rewritten to 720x560 the window
    # grew past its own gate and the run failed on containment. The box is now
    # computed from `design::GEOMETRY` at the top of launcher.sh, so what this
    # check can pin is that each gate is still *passed* one -- the shape of the
    # box is the derivation's business, and `the box is derived` below asserts
    # the derivation exists.
    EXPECT_BOX = '"${expect_box[@]}"'
    wanted = {
        "the open gate": ("launcher-01-open.png", "--min-percent 3",
                          EXPECT_BOX, "--ignore-box 0 0 1279 139"),
        "the went-away gate": ("launcher-04-hidden.png", "--max-percent 3",
                               "--ignore-box 0 0 1279 139",
                               "--ignore-box 0 700 1279 799"),
        "the came-back gate": ("launcher-05-summoned.png", "--min-percent 3",
                               EXPECT_BOX, "--ignore-box 0 0 1279 139"),
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

    # The gates above are only as good as the box they are handed, and a box
    # built from nothing would satisfy every check above. So the derivation is
    # pinned too: launcher.sh must read the window size out of design.rs rather
    # than carry its own copy of it, which is the mistake that made the literal
    # box go stale in the first place.
    derivation = (
        # The whole expression, not the `expect_box=(--expect-box` prefix: a
        # control that replaced the computed values with the old literals kept
        # that prefix and this check stayed silent, which made it a test of
        # nothing. The variables are what say the box was computed.
        ('expect_box=(--expect-box "$box_x0" "$box_y0" "$box_x1" "$box_y1")',
         "launcher.sh does not build expect_box from the computed corners"),
        ("card_width", "launcher.sh does not read card_width from the design tokens"),
        ("card_max_height", "launcher.sh does not read card_max_height from the design tokens"),
        ("design.rs", "launcher.sh does not name design.rs as the source of the geometry"),
    )
    _check_box_derivation(launcher)

    for needle, complaint in derivation:
        if needle not in launcher:
            raise SystemExit(
                f"{complaint} (looked for {needle!r}).\n"
                "  The containment box must be derived from design::GEOMETRY: a box written "
                "down here goes stale the next time the card is resized, which is #100's "
                "launcher-gate failure.\n"
                "  Update the gates in this file and re-run the controls."
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


def _check_box_derivation(launcher: str) -> None:
    """Run launcher.sh's box derivation and check the box against the window.

    The text checks below can see that the box is built from variables; they
    cannot see whether those variables are computed. A control that froze one
    corner (`box_x1=980`) left every one of them green, which made them a test
    of spelling rather than of behaviour.

    So the derivation is extracted and executed against real and edited design
    tokens, and what is asserted is the property that matters: the box contains
    the window, centred, with the top clamped to the ignored strip. A frozen
    corner fails the moment the window moves away from it.
    """
    import pathlib
    import re
    import subprocess
    import tempfile

    lines = launcher.splitlines()
    try:
        start = next(i for i, line in enumerate(lines) if line.startswith("screen_w="))
        end = next(i for i, line in enumerate(lines) if line.startswith("expect_box="))
    except StopIteration:
        raise SystemExit(
            "launcher.sh no longer has a `screen_w=`..`expect_box=` derivation block, so the "
            "containment box cannot be checked. Update the gates in this file and re-run the "
            "controls."
        ) from None

    block = "\n".join(lines[start : end + 1])
    root = pathlib.Path(__file__).resolve().parents[2]
    design = root / "crates/compass-ui/src/design.rs"
    source = design.read_text(encoding="utf-8")

    def derive(width: int, height: int) -> tuple[int, int, int, int]:
        edited = re.sub(r"card_width: \d+,", f"card_width: {width},", source, count=1)
        edited = re.sub(r"card_max_height: \d+,", f"card_max_height: {height},", edited, count=1)
        with tempfile.TemporaryDirectory() as tmp:
            staged = pathlib.Path(tmp) / design.relative_to(root)
            staged.parent.mkdir(parents=True, exist_ok=True)
            staged.write_text(edited, encoding="utf-8")
            script = f"{block}\necho \"$box_x0 $box_y0 $box_x1 $box_y1\"\n"
            done = subprocess.run(
                ["bash", "-euo", "pipefail", "-c", script],
                cwd=tmp,
                capture_output=True,
                text=True,
            )
        if done.returncode != 0:
            raise SystemExit(
                f"launcher.sh's box derivation failed for a {width}x{height} window:\n"
                f"{done.stderr.strip()}\n  Update the gates in this file and re-run the controls."
            )
        return tuple(int(value) for value in done.stdout.split())

    screen_w, screen_h, top_bar = 1280, 800, 140
    for width, height in ((720, 560), (640, 480), (480, 360)):
        x0, y0, x1, y1 = derive(width, height)
        win_x0, win_x1 = (screen_w - width) // 2, (screen_w + width) // 2
        win_y0, win_y1 = (screen_h - height) // 2, (screen_h + height) // 2
        problems = []
        if x0 > win_x0 or x1 < win_x1:
            problems.append(f"does not contain the window's {win_x0}..{win_x1} horizontally")
        if y0 > max(win_y0, top_bar) or y1 < win_y1:
            problems.append(f"does not contain the window's {win_y0}..{win_y1} vertically")
        if problems:
            raise SystemExit(
                f"for a {width}x{height} window centred on {screen_w}x{screen_h}, launcher.sh "
                f"derives the containment box {x0},{y0}..{x1},{y1}, which "
                + " and ".join(problems)
                + ".\n  That is #100's launcher-gate failure: a box that does not track the "
                "window fails every run once the card is resized.\n"
                "  Update the gates in this file and re-run the controls."
            )


if __name__ == "__main__":
    sys.exit(main())

