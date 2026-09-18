#!/usr/bin/env bash
# Drive the launcher: open it in the guest, look at the screen, type at it.
#
# Usage: scripts/vmtest/launcher.sh <artifact-dir> <vm-name>
#
# WHY THIS IS NOT A `--check`
#
# Same reason as spike-a.sh. The question here is "did a launcher appear on
# screen", and the only observer of the framebuffer is corral, on the host side
# of QEMU. Nothing inside the guest can answer it — ADR-0010 says so and it is
# still true. So the sequence straddles guest and host, which means running
# after `corral vmtest` returns, against a VM it was told to leave up.
#
# WHAT IS GATED, AND WHAT IS ONLY RECORDED
#
# Gated: the launcher process starts and stays up (checks.sh, in the guest),
# and the screen still passes corral's own blank test with the launcher open.
# That threshold is corral's, not one invented here.
#
# Recorded, not gated: the three luminance deviations. It is tempting to assert
# that opening a launcher raises the deviation, and it probably does — but this
# has never been measured once, and ADR-0010 is explicit that inventing a pixel
# threshold before seeing real numbers is how a tier starts flaking. Spike A and
# Spike B both shipped gating on nothing but "produced a report", for the same
# reason. Once a few runs have published numbers, the gate can be set from data
# rather than from a guess, and that is a two-line change to this file.
set -euo pipefail

out="${1:?usage: launcher.sh <artifact-dir> <vm-name>}"
vm="${2:?usage: launcher.sh <artifact-dir> <vm-name>}"
query="${LAUNCHER_QUERY:-fi}"
checks=/usr/libexec/compass-vmtest/checks.sh

# corral wrote result.json as root and it holds the run's own SSH details.
# Reading them beats rebuilding the invocation here and drifting out of step.
sudo chown -R "$(id -un)" "$out" 2>/dev/null || true
mapfile -t ssh_argv < <(RESULT="$out/result.json" python3 - <<'PY'
import json, os
ssh = json.load(open(os.environ['RESULT']))['ssh']
for part in ['ssh',
             '-i', ssh['identityFile'],
             '-p', str(ssh['port']),
             '-o', 'StrictHostKeyChecking=no',
             '-o', 'UserKnownHostsFile=/dev/null',
             '-o', 'LogLevel=ERROR',
             '-o', 'BatchMode=yes',
             f"{ssh['user']}@{ssh['host']}"]:
    print(part)
PY
)

guest() { "${ssh_argv[@]}" "$@"; }

# `sudo -E "$(command -v corral)"`, not `sudo corral`: corral is on PATH via
# GITHUB_PATH, and sudo replaces PATH with secure_path, so `sudo corral` is
# "command not found". Verified that -E alone does not help — secure_path wins.
corral_bin="$(command -v corral)"
shot() { sudo -E "$corral_bin" screenshot "$vm" -o "$out/$1" ; }

echo "=== 0. what the session is actually showing at login ==="
# Evidence only, and kept separate from the bare desktop below because they
# turned out not to be the same picture.
shot "launcher-00z-login.png"

echo
echo "=== 0a. put the session on a bare desktop (the Activities overview) ==="
# GNOME Shell opens the overview at login when the session has no windows.
# Until the first-run tour was suppressed the session always had one, so this
# never came up; the first run without the tour shows what it costs. Three
# things in that run's own frames, none of which the gates could name:
#
#   launcher-01-open.png    our window rendered as a SCALED THUMBNAIL inside a
#                           workspace tile, because it opened behind the
#                           overview. The gate at 3d passed on it -- at 34% in
#                           the expected box -- while measuring a preview.
#   launcher-04-hidden.png  the query from step 3 typed into GNOME's OWN search
#                           field, with GNOME's own results under it.
#   the 3d4 assertion       81.60% against a 3% bound, which is what finally
#                           failed, and the only reason any of this was seen.
#
# Dismissed in the guest over the session bus (checks.sh overview-dismiss), not
# by injecting Escape from the host, so the step can read back whether it
# worked instead of hoping.
guest "$checks" overview-dismiss

echo
echo "=== 0a2. the bare desktop, before anything of ours runs ==="
# Evidence only. The control for every gate below is step 0c, not this.
shot "launcher-00a-bare-desktop.png"

echo
echo "=== 0b. start the engine, so the launcher has something to attach to ==="
# ADR-0015 made the window resident and driven: `vicinae ui` connects to
# `vicinae serve` and waits to be told to show. Order matters -- a launcher
# started first comes up undriven, and every summon below is then refused
# correctly and confusingly.
guest "$checks" engine-start

echo
echo "=== 0c. the desktop with the engine up (THE CONTROL for every gate) ==="
# THE CONTROL MOVED, AND THAT IS THE POINT.
#
# It used to be taken before anything of ours ran, when the only thing that
# happened between it and the launcher frame was the launcher. Starting the
# engine in between broke that: the gate at 3d asks "did a LAUNCHER window
# appear", and it was being shown every pixel the engine changed as well.
#
# It cost a run. The gate failed with a changed region 962 px wide against a
# 640 px window -- while the launcher's own log showed it configuring a surface
# at exactly 640x480, so the window was never the problem. `serve` binds the
# GlobalShortcuts portal at startup, and the portal asks the user for
# permission; whatever that puts on screen was landing inside a comparison that
# claimed to be about the launcher.
#
# Taking the control AFTER the engine is up restores what the gate means: the
# only thing that differs between this frame and the next is the launcher. The
# thresholds and boxes below are untouched -- this is not a gate loosened to
# fit a failure, it is a control put back where it belongs.
shot "launcher-00-before.png"

echo
echo "=== 0d. starting the engine must not put anything on screen (the gate) ==="
# THIS WAS A RECORDING AND IS NOW A GATE, on the strength of what it recorded.
#
# It was added ungated to answer a question two failed runs could only infer:
# what does starting the engine draw? The answer came back 1.62% of pixels in a
# 496x532 box -- GNOME asking the user to grant the launcher hotkey, which
# `serve` requests from the GlobalShortcuts portal at startup.
#
# So `engine-start` now passes `--no-hotkey`, and with nothing to ask about,
# starting the engine should be invisible. That makes this a real assertion
# about the flag: if it stops working, the dialog comes back and this fails
# here, naming the cause, instead of corrupting the launcher gate three steps
# later with a bounding box nobody can interpret.
#
# TWO STRIPS OF SHELL FURNITURE ARE EXCLUDED, top and bottom, and neither is
# ours to keep still.
#
# The top bar carries a clock, and a minute boundary is not the engine drawing
# something -- that one cost a red run back when the launcher gate was written.
#
# The bottom is the dash, and it cost this gate its first run: 2.04% in a
# 237x84 box along the bottom edge. Cropping both frames to that region and
# looking at them shows the dash's ICONS are identical -- Files, Trash, the app
# grid, unchanged -- and what moves is the panel backdrop behind them, which
# the shell redraws when a process starts. Nothing we can prevent and nothing
# worth failing over.
#
# What is left between the strips is the part that matters: if the hotkey
# dialog ever comes back it lands in the middle of the screen, where this gate
# is still sharp.
#
# Binding the hotkey for real is Spike A's job, in its own VM, where a dialog
# is the expected outcome rather than contamination.
python3 scripts/vmtest/framediff.py \
  "$out/launcher-00a-bare-desktop.png" "$out/launcher-00-before.png" \
  --max-percent 1 --ignore-box 0 0 1279 139 --ignore-box 0 700 1279 799

echo
echo "=== 1. open the launcher in the session ==="
guest "$checks" launcher-start

echo
echo "=== 1b. what is the launcher blocked on? ==="
# Added after the instrumented run: the log stops 200ms in and the process sits
# there alive, so logging cannot say where it stopped — the stuck code is not
# the code doing the logging. The kernel can. Best-effort; a diagnostic must
# never fail the run it exists to explain.
guest "$checks" launcher-diagnose || true

echo
echo "=== 2. the screen with the launcher open ==="
sudo -E "$corral_bin" screenshot "$vm" -o "$out/launcher-01-open.png" --require-paint

echo
echo "=== 3. type a query at it ==="
# THIS STEP NOW ASSERTS SOMETHING, and the history of why it did not is worth
# keeping, because the comment has been wrong twice and each correction was paid
# for by a run.
#
# It first said injection "does not reach this session at all"; the evdev
# capture disproved that, and the chain is longer:
#
#   QEMU delivers the scancode          yes — 288 bytes captured on the evdev
#                                        node while the key was injected
#   the guest kernel sees it            yes — same evidence
#   gnome-shell holds the keyboard      yes — event0..event3 open on its fds
#   the portal grants the binding       yes — "portal available | bind granted"
#   the shortcut fires                  NO  — "activated False"
#
# So the loss is above the kernel, in a session whose compositor is holding the
# device it is losing events from. "Does not reach the session" would point at
# corral or QEMU, which the capture rules out.
#
# The seat is ruled out too, as of the run that first printed the untruncated
# diagnostic: the AT keyboard IS on seat0, and the capture decodes clean
# LEFTMETA press and release on event1 -- the very node gnome-shell holds open.
#
# THE PARAGRAPH THAT USED TO BE HERE WAS WRONG, and the run that suppressed the
# first-run tour disproved it with a screenshot.
#
# It argued from "pressing Super ALONE leaves the framebuffer byte-identical"
# that "the compositor is inert to injected input generally", and went on to
# suspect logind of having the session's devices paused -- which would have been
# a deep problem and was not one. In that run's `launcher-04-hidden.png` the
# query from THIS STEP is sitting in GNOME Shell's own search field with GNOME's
# own results rendered under it. The compositor receives injected keys and acts
# on them.
#
# What was actually inert was the session, because the tour dialog held the
# keyboard: Super does nothing while a modal dialog has the grab, and the
# framebuffer stays byte-identical for that reason rather than for the one the
# paragraph gave. Every layer in the table above was measured correctly; only
# the last line's cause was wrong.
#
# So "activated False" for our own shortcut is still unexplained and is still
# Spike A's question -- but it is no longer explained by input not arriving, and
# the next person should not spend a run on logind.
#
# With the overview dismissed and the tour gone, the keys reach whatever holds
# the focus, and our window is the only window on the desktop. On the run that
# first got there, `launcher-02-typed.png` came back BYTE-IDENTICAL to
# `launcher-01-open.png`: the launcher drew a search box and ignored the
# keyboard. That was #91, and the cause was in our code -- Iced delivers typed
# characters only to a `text_input` that holds widget focus, and nothing ever
# focused ours. A person trying the launcher clicks the box without noticing
# they did; the tier only types, which is why the tier found it.
#
# So the gate below is the regression check for that fix, and the floor is
# deliberately near zero rather than fitted. What is being asserted is "the
# keystroke reached our field at all", which is threshold-free; how many pixels
# two characters and a results list move has not been measured yet, and ADR-0010
# is explicit that inventing a number before seeing one is how a tier starts
# flaking. The run prints the real figure, and the floor can be raised from data
# in a two-line change once a few runs agree.
#
# The box and the ignored strips are the launcher gate's, for the same reasons
# given there: the region that may change is the window, and the clock and the
# dash are not ours to keep still.
#
# CONTROLLED, on the published artifact of run 35285035604 rather than on a
# claim: these exact arguments against that run's two frames exit 1 with
# "FAIL: only 0.00% of pixels changed, expected at least 0.10%".
sudo -E "$corral_bin" type "$vm" "$query"
shot "launcher-02-typed.png"

echo
echo "=== 3a. did the query reach OUR field? (the gate, #91) ==="
python3 scripts/vmtest/framediff.py \
  "$out/launcher-01-open.png" "$out/launcher-02-typed.png" \
  --min-percent 0.1 --expect-box 300 140 980 800 \
  --ignore-box 0 0 1279 139 --ignore-box 0 700 1279 799

echo
echo "=== 3b. and is it blocked in the same place after the keystroke? ==="
# The same probe again, deliberately. A process parked in the same syscall on
# the same socket both times is stuck; one that has moved is merely slow, and
# those two want completely different fixes.
guest "$checks" launcher-diagnose || true

echo
echo "=== 3b2. what does the launcher cost at idle? (Phase 1 gate) ==="
# Phase 1's gate says "idle RSS < 30 MB" and nobody had measured it, because
# until the launcher drew there was nothing to measure. Taken here, after the
# window is up and before the control application starts competing for memory.
guest "$checks" launcher-rss || true

echo "=== 3d. did a launcher window actually appear? (the gate) ==="
# The first assertion in this tier derived from a measurement rather than from
# an assumption, and the reason it exists is that its absence let a wrong
# conclusion stand for three runs.
#
# corral's luminance deviation cannot answer "did a window appear": five frames
# across two jobs all read 0.1564 to four decimal places while showing visibly
# different things, one of them containing a 640x480 launcher. Pixels can, and
# framediff.py counts them.
#
# The numbers are taken from two consecutive runs that agreed to the pixel:
# 84150 changed (8.22%) in a box at x 335..942, y 152..796, against a window
# configured 640x480 centred, i.e. x 320..960, y 160..640. The gate is set well
# below and around that — 3% rather than 8.22%, and a box with room on every
# side — because the point is to catch "nothing was drawn", not to pin the
# exact pixels of a theme. A tighter bound would break on the first font change
# and teach everyone to ignore it.
#
# THE TOP BAR IS IGNORED, and that is not a loosening of the gate.
#
# --expect-box asserts the changed region lies WITHIN the box, which silently
# also asserts that nothing else on the screen changed between the two frames.
# That was never the intent, and it is not true: GNOME's top bar carries a
# clock. It cost a red run to find out, and the two runs say exactly what
# happened:
#
#   passing   84077 changed  box x 335..942  y 152..796
#   failing   84172 changed  box x 335..942  y  10..796
#
# Ninety-five extra pixels, in the top bar, with the launcher's own footprint
# identical to the pixel on both axes. A clock digit turning over dragged miny
# from 152 to 10 and failed a gate about whether a window appeared. Two
# calibration runs happened not to cross a minute boundary; this one did.
#
# So the shell's own furniture is excluded from the comparison rather than the
# box being widened to swallow it. Widening would have made "the launcher
# painted at the top of the screen" pass, which is a real failure this gate
# should keep catching. Excluding y < 140 keeps the containment assertion sharp
# for the whole region the launcher can legitimately occupy, and --min-percent
# still requires a real window's worth of pixels inside it: a launcher that
# painted only in the ignored strip would now change ~0% and fail there.
python3 scripts/vmtest/framediff.py \
  "$out/launcher-00-before.png" "$out/launcher-01-open.png" \
  --min-percent 3 --expect-box 300 140 980 800 --ignore-box 0 0 1279 139

echo
echo "=== 3d2. can the engine hide and summon the window? (the ADR-0015 gate) ==="
# THE THING THIS TIER COULD NOT ASSERT BEFORE.
#
# Every check above can see a *process*. None of them can see a *connection*.
# `serve` refuses show/hide/toggle when no window has attached, so a `toggle`
# that succeeds is proof of the whole chain at once: CLI, socket, engine, the
# window link, and a window that answered on the other end of it.
#
# The keypress leg is still missing and is still not this job's to fix:
# injected input does not reach this compositor (see step 3's note), so the
# client here is `vicinae` rather than Super+Space. What that leaves untested
# is the portal delivering an activation. Everything after the activation is
# exercised.
guest "$checks" window-attached
shot "launcher-04-hidden.png"

echo
echo "=== 3d3. and does summoning it bring the window back? ==="
guest "$checks" summon
# The same settle the open path gets. Opening a surface under llvmpipe is not
# instant, and screenshotting before the paint would produce "summon does not
# work" for the same reason step 1 produced "the launcher does not draw" --
# twice, wrongly.
sleep 5
sudo -E "$corral_bin" screenshot "$vm" -o "$out/launcher-05-summoned.png" --require-paint

echo
echo "=== 3d4. the two assertions that make the pair mean something ==="
# Hidden must look like the bare desktop, and summoned must look like the
# launcher again. Either alone is weak: a frame that never changes passes the
# first, and a frame that never changes fails the second, so the pair together
# is what says the window actually went away and actually came back.
#
# Same box and the same ignore strip as the gate above, for the same reasons --
# including the top-bar clock, which cost a red run to discover.
echo "--- the window went away: hidden should match the desktop before it opened ---"
# Same two strips as step 0d, for the same reason: the dash gains an entry
# while the launcher process is resident, and stays changed after its window is
# gone. That is correct -- the process really is still running, which is the
# whole point of ADR-0015 -- so it must not be read as "the window is still
# there".
python3 scripts/vmtest/framediff.py \
  "$out/launcher-00-before.png" "$out/launcher-04-hidden.png" \
  --max-percent 3 --ignore-box 0 0 1279 139 --ignore-box 0 700 1279 799

echo "--- and came back: summoned should look like the launcher did ---"
python3 scripts/vmtest/framediff.py \
  "$out/launcher-00-before.png" "$out/launcher-05-summoned.png" \
  --min-percent 3 --expect-box 300 140 980 800 --ignore-box 0 0 1279 139

echo
echo "=== 3d5. what did the engine make of the hotkey? (recorded, not gated) ==="
# Whether GNOME grants LOGO+space is the user's decision through a permission
# dialog, and an unattended session may well be refused. That is a real outcome
# worth reading in the log, not a failure of the code.
guest "$checks" hotkey-status || true

echo
echo "=== 3d6. can ANY client draw in this session? (the control) ==="
# MOVED AFTER THE SUMMON GATES, AND THE ORDER IS THE POINT. This starts a
# second application and leaves it on screen. Run before the gates above, it
# puts a whole file manager into frames that are compared against a control
# taken before it existed -- which is exactly how 3d4 failed at 20.38% in a
# 957px box, with a nautilus window sitting in the "hidden" frame.
#
# It is a diagnostic for 3d, not an input to it, so it loses nothing by
# running once the frame-comparing steps are done.
# The control this job should have had from the start. Everything above says
# our launcher puts no window on screen; none of it distinguishes that from
# nothing being able to. A session where no client can render would produce
# identical evidence and would exonerate the launcher entirely — and the
# desktop painting does not settle it, because that is GNOME Shell compositing
# its own furniture, not a client surface.
guest "$checks" control-app-start || true
shot "launcher-03-control-app.png"

echo
echo
echo "=== 3e. harvest a real desktop-entry corpus from this box ==="
# Phase 1's gate wants ~500 real entries for Suite 0 ranking parity and the
# corpus has 27, which §11.2 calls the binding constraint on the phase. The
# harvester asks for "a real desktop — ideally a Bluefin box"; this VM is one,
# booted fresh with a full GNOME application set on the target platform.
#
# It produces an artifact for a person to review and commit. It deliberately
# does not write into the repository: a corpus shapes every ranking assertion
# we make, and it should not grow by a job quietly appending to it.
guest "$checks" harvest-corpus || true
if "${ssh_argv[@]}" test -f /tmp/corpus.tar.gz 2>/dev/null; then
  # `ssh cat > file` rather than scp: scp's protocol has been deprecated and
  # removed in places, and this needs no second tool.
  "${ssh_argv[@]}" cat /tmp/corpus.tar.gz > "$out/corpus.tar.gz" 2>/dev/null \
    && echo "corpus.tar.gz retrieved ($(wc -c < "$out/corpus.tar.gz") bytes)" \
    || echo "(could not retrieve the corpus tarball)"
else
  echo "(no corpus tarball in the guest)"
fi

echo
echo "=== 4. is it still running? ==="
# A launcher that dies on the first keystroke is a real bug, and without this
# it would show up only as two screenshots that happen to look alike.
guest "$checks" launcher-status
