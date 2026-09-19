#!/usr/bin/env bash
# The VM tier's assertions about compass, run inside the guest.
#
# These are `corral vmtest --check` commands, which arrive over SSH as root, one
# fresh shell each. They live in a script baked into the test image rather than
# as shell one-liners in the workflow for three reasons: `bash -n` can check
# them before CI does, quoting them through YAML and ssh twice is how subtle
# bugs get in, and the shape of "wait for X, then assert Y" does not survive
# being folded onto one line.
#
# Each subcommand is one assertion, so a failing run names which one broke
# instead of reporting that a compound command exited 1. corral runs every check
# even after one fails, so the report is the whole picture.
set -euo pipefail

APP=com.vicinae.Vicinae
INSTALLATION=compass
SESSION_USER=compass
REPORT=/tmp/compass-doctor.json
SPIKE_OUT=/tmp/compass-spike-a.json
SPIKE_ERR=/tmp/compass-spike-a.err
SPIKE_DONE=/tmp/compass-spike-a.done
UI_ERR=/tmp/compass-ui.err
UI_DONE=/tmp/compass-ui.done
ENGINE_ERR=/tmp/compass-engine.err
ENGINE_DONE=/tmp/compass-engine.done
ENGINE_PIDS=/tmp/compass-engine.pids
CONTROL_ERR=/tmp/compass-control-app.err
CONTROL_DONE=/tmp/compass-control-app.done
KBD_CAP_DIR=/tmp
KBD_CAP=/tmp/compass-kbd-capture.bin
KBD_CAP_DONE=/tmp/compass-kbd-capture.done

# uid of the autologin user. Everything about a session is addressed by it.
uid() { id -u "$SESSION_USER"; }

# wait_for runs a command until it succeeds, up to a timeout. Used instead of a
# fixed sleep because llvmpipe makes GNOME's timing wildly variable — ADR-0010
# says to key assertions off states, never off durations.
wait_for() {
  local what="$1" seconds="$2"; shift 2
  local deadline=$((SECONDS + seconds))
  until "$@"; do
    if (( SECONDS >= deadline )); then
      echo "timed out after ${seconds}s waiting for: $what" >&2
      return 1
    fi
    sleep 2
  done
  echo "$what: after ${SECONDS}s"
}

# True when some process is running the named binary, or when the sentinel says
# it already exited. Compares the resolved /proc/PID/exe rather than matching a
# command line, so it cannot match the shell that is doing the asking.
exe_running() {
  local want="$1" sentinel="$2" exe
  [ -f "$sentinel" ] && return 0
  for d in /proc/[0-9]*; do
    exe="$(readlink "$d/exe" 2>/dev/null || true)"
    case "$exe" in
      */"$want") return 0 ;;
    esac
  done
  return 1
}

# Run a compass CLI command as the session user, inside the Flatpak.
#
# Named `compass_cli`, not `as_session`: `spike-a-evidence` already defines a
# local `as_session` that runs an arbitrary guest binary without the Flatpak,
# and two helpers of the same name doing different things is how the wrong one
# gets called.
#
# Every summon check needs the same seven environment variables, and getting
# XDG_RUNTIME_DIR wrong means talking to a socket that is not the session's --
# which presents as "no engine running" rather than as a mistake here.
#
# THE ARGUMENTS ARE THE CLI'S OWN SUBCOMMAND, with no `vicinae` in front. The
# Flatpak's entrypoint IS `vicinae`, so `compass_cli vicinae ping` runs
# `vicinae vicinae ping` -- rejected by the parser, forever. That cost a
# 30-minute VM run, presenting as "the engine never answered a ping" with a
# perfectly healthy engine sitting there. `crates/vicinae/tests/vmtest_cli.rs`
# now parses these call sites with the real clap definition so the next one
# fails in seconds instead.
compass_cli() {
  local u
  u="$(uid)"
  runuser -u "$SESSION_USER" -- env \
    XDG_RUNTIME_DIR="/run/user/$u" \
    DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/$u/bus" \
    WAYLAND_DISPLAY="$(wayland_display)" \
    XDG_SESSION_TYPE=wayland \
    flatpak run --installation="$INSTALLATION" "$APP" "$@"
}

# The launcher's pid, excluding the engine's.
#
# BOTH PROCESSES ARE NAMED `vicinae`. Before ADR-0015 there was only ever one,
# and `pgrep -u "$SESSION_USER" -x vicinae | head -1` was unambiguous. Now
# `serve` runs first, so that pgrep matches the engine -- and matching the
# engine is not a cosmetic problem:
#
#   * `launcher-start`'s waiter would be satisfied the instant it began,
#     because a `vicinae` process already exists, so it would stop waiting for
#     the launcher entirely;
#   * `launcher-diagnose` would report which syscall the *engine* is parked in
#     while claiming to explain the launcher;
#   * `launcher-rss` would measure the wrong process against Phase 1's gate.
#
# So the engine records its pids and everything about the launcher skips them.
# Matching on the command line instead is the obvious alternative and is worse
# here: both are `flatpak run … com.vicinae.Vicinae <verb>` and the inner
# process is a grandchild whose argv is not the one written above.
launcher_pid() {
  local engine="" pid
  [ -f "$ENGINE_PIDS" ] && engine="$(tr '\n' ' ' < "$ENGINE_PIDS")"
  for pid in $(pgrep -u "$SESSION_USER" -x vicinae 2>/dev/null); do
    case " $engine " in
      *" $pid "*) continue ;;
    esac
    echo "$pid"
    return 0
  done
  return 1
}

# True once the launcher exists, or once it has exited.
launcher_appeared() {
  launcher_pid >/dev/null || [ -f "$UI_DONE" ]
}

# True once the engine answers, or once it has exited.
#
# A shell function rather than a `bash -c` predicate, because `wait_for` calls
# what it is given in the current shell: a subshell would not inherit
# `compass_cli`, `uid` or `APP`, and the check would fail for reasons that have
# nothing to do with the engine.
engine_ready() {
  compass_cli ping >/dev/null 2>&1 || [ -f "$ENGINE_DONE" ]
}

# The session's Wayland socket name. Read from the runtime directory rather than
# assumed to be wayland-0: it is whatever the compositor bound, and guessing
# would make session.type fail for a reason that has nothing to do with us.
wayland_display() {
  local dir
  dir="/run/user/$(uid)"
  find "$dir" -maxdepth 1 -name 'wayland-[0-9]*' -printf '%f\n' 2>/dev/null | sort | head -1
}

case "${1:?usage: checks.sh <subcommand>}" in

  # 1. The Flatpak is in the image at all. If this fails, the layering in
  #    Containerfile.compass is wrong and nothing below can mean anything.
  installed)
    flatpak --installation="$INSTALLATION" list --app --columns=application,version,branch
    flatpak --installation="$INSTALLATION" list --app --columns=application | grep -qx "$APP"
    ;;

  # 2. GDM autologin reached a real user session. A greeter would still paint,
  #    still answer SSH and still pass --require-paint, so this is the assertion
  #    that separates "the OS booted" from "a user is logged in" — and every
  #    check below is meaningless without it.
  session)
    wait_for "a session for $SESSION_USER" 180 \
      bash -c 'loginctl list-sessions --no-legend | grep -qw compass'
    wait_for "the user session bus" 120 test -S "/run/user/$(uid)/bus"
    wait_for "a wayland socket" 120 bash -c '[ -n "$(find "/run/user/$(id -u compass)" -maxdepth 1 -name "wayland-[0-9]*" -print -quit 2>/dev/null)" ]'
    loginctl list-sessions --no-legend
    loginctl show-session "$(loginctl list-sessions --no-legend | awk '$3=="compass"{print $1; exit}')" \
      -p Type -p State -p Active -p Remote || true
    ;;

  # 3. Run the diagnostic from inside the session, as the user, with the session's
  #    own environment — which is the entire point of this tier. The same binary
  #    runs in the Flatpak CI job, but there it runs in a container with no
  #    desktop, so half its checks cannot say anything true.
  #
  #    This subcommand does not assert; it produces the evidence. `doctor` without
  #    --check-only always exits zero by design, so a pass here means only that
  #    the sandboxed binary started in a real session. That is worth knowing on
  #    its own, and the assertions are the next subcommand.
  doctor)
    u="$(uid)"
    runuser -u "$SESSION_USER" -- env \
      XDG_RUNTIME_DIR="/run/user/$u" \
      DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/$u/bus" \
      WAYLAND_DISPLAY="$(wayland_display)" \
      XDG_SESSION_TYPE=wayland \
      flatpak run --installation="$INSTALLATION" "$APP" \
        --socket /tmp/compass-vmtest.sock doctor --json > "$REPORT"
    cat "$REPORT"
    ;;

  # 4. The claims worth gating on.
  #
  #    Only three, and each is a fact about the target platform that no other
  #    tier can establish:
  #      session.type    — a Wayland session exists and the sandbox can see it
  #      dbus.session    — the session bus is reachable from inside the sandbox
  #      portal.desktop  — xdg-desktop-portal answers there
  #    Everything else is printed and gated on nothing. In particular
  #    portal.global-shortcuts and gnome.shell-extension are evidence only: the
  #    first is Spike A's subject and not yet expected to work, and this
  #    repository ships no Shell extension for the second to find (ADR-0004).
  doctor-assert)
    REPORT="$REPORT" python3 - <<'PY'
import json, os, sys

report = json.load(open(os.environ['REPORT']))
checks = report.get('checks') or []
if not checks:
    sys.exit('doctor produced no checks')

status = {c['name']: c['status'] for c in checks}
for c in checks:
    print(f"  {c['status']:5} {c['name']}: {c.get('detail', '')}")

required = ['session.type', 'dbus.session', 'portal.desktop']
bad = [f"{n}={status.get(n, 'MISSING')}" for n in required if status.get(n) != 'ok']
if bad:
    sys.exit('not ok in a real GNOME session: ' + ', '.join(bad))
print('\nall gated checks ok:', ', '.join(required))
PY
    ;;

  # Spike B (#3): the same question on the target kernel. The Flatpak CI job
  # answers it in three minutes on the runner's kernel; this one answers it on
  # Bluefin's, which is what actually ships, and Landlock's ABI is a kernel
  # property. Evidence only — no assertion, because every outcome is a finding.
  spike-b)
    u="$(uid)"
    runuser -u "$SESSION_USER" -- env \
      XDG_RUNTIME_DIR="/run/user/$u" \
      DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/$u/bus" \
      flatpak run --installation="$INSTALLATION" "$APP" spike sandbox
    ;;

  # ── Spike A ────────────────────────────────────────────────────────────────
  #
  # Two halves, because a host-side keypress has to happen between them. corral
  # runs every --check over its own SSH connection and cannot interleave a host
  # command, so Spike A is driven by scripts/vmtest/spike-a.sh after vmtest
  # returns, against a VM left running.

  # Start the spike detached and return once it says it is listening. Returning
  # earlier would race: binding is a portal round trip and, on a first run, a
  # permission dialog, and a key sent before the bind lands proves nothing.
  spike-a-start)
    u="$(uid)"
    # Created empty rather than removed: the waiter greps the stderr file, and
    # a file that does not exist yet makes grep print "No such file or
    # directory" into a log where it reads like the failure rather than like
    # the first poll of a loop that then succeeded.
    : > "$SPIKE_OUT"
    : > "$SPIKE_ERR"
    rm -f "$SPIKE_DONE"

    # setsid and all three fds redirected: without that, ssh waits for the
    # channel to close and this check never returns.
    #
    # The wrapper exists to write $SPIKE_DONE when the spike exits. The obvious
    # alternative — having the collector poll `pgrep -f "spike global-shortcut"`
    # — cannot work, and failed exactly this way: pgrep matches full command
    # lines, so the shell running the pgrep contains the pattern and matches
    # itself. The predicate is then never true and the wait always times out.
    # A sentinel file has no such reflexivity, and it carries the exit status.
    #
    # Arguments are passed positionally rather than interpolated, so nothing
    # here depends on quoting surviving two levels of shell.
    setsid bash -c '
      runuser -u "$1" -- env \
        XDG_RUNTIME_DIR="/run/user/$2" \
        DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/$2/bus" \
        WAYLAND_DISPLAY="$3" \
        XDG_SESSION_TYPE=wayland \
        flatpak run --installation="$4" "$5" \
          spike global-shortcut --trigger "$6" --wait 120 --json \
        > "$7" 2> "$8"
      echo "$?" > "${9}"
    ' _ "$SESSION_USER" "$u" "$(wayland_display)" "$INSTALLATION" "$APP" \
      "${2:-SUPER+space}" "$SPIKE_OUT" "$SPIKE_ERR" "$SPIKE_DONE" \
      < /dev/null >> "$SPIKE_ERR" 2>&1 &

    # Wait for the marker OR the spike exiting, not the marker alone. A spike
    # that cannot reach the portal writes its report and exits, and waiting only
    # for the marker means sitting out the full timeout and then discarding an
    # answer that already existed. (The spike now announces readiness on every
    # path, so this is belt and braces — but the belt is what turns a hang into
    # a report, and it costs one `-f` test.)
    wait_for "the spike to bind and start listening, or exit" 150 \
      bash -c 'grep -q SPIKE-A-READY "$1" || [ -f "$2" ]' _ "$SPIKE_ERR" "$SPIKE_DONE"

    if ! grep -q SPIKE-A-READY "$SPIKE_ERR"; then
      echo "the spike exited before announcing readiness; its report follows in the next step"
    fi
    cat "$SPIKE_ERR"
    ;;

  # Wait for the spike to finish — it exits on the first activation, or at its
  # own deadline — and print the report. Deliberately does NOT assert that the
  # shortcut fired: "GNOME refused to bind without a click nobody can give" is
  # an answer to the question, not a broken run, and a gate here would turn the
  # finding into a red X with no information in it.
  spike-a-collect)
    wait_for "the spike to finish" 180 test -f "$SPIKE_DONE"
    echo "the spike exited $(cat "$SPIKE_DONE")"
    echo '--- stderr ---'
    cat "$SPIKE_ERR" 2>/dev/null || echo '(none)'
    echo '--- report ---'
    cat "$SPIKE_OUT"
    python3 -c "import json,sys; json.load(open('$SPIKE_OUT'))" \
      || { echo 'the spike produced no valid JSON report' >&2; exit 1; }
    ;;

  # Does the compositor have the keyboard open?
  #
  # The evdev capture settled that scancodes reach the kernel — 288 bytes, 12
  # input_event structs, exactly two press/release pairs. And they reach
  # neither GNOME's Super binding nor a focused application: typing into the
  # launcher's own text input left the frame byte-identical.
  #
  # Keys arriving at the kernel and being acted on by nobody points at one
  # thing worth checking before any theory: whether gnome-shell has the device
  # open at all. libinput reads evdev nodes directly, so if the compositor's
  # process holds no /dev/input fd, it is not reading the keyboard and nothing
  # above that matters.
  #
  # This is a fact, not an inference, and it costs one readlink loop.
  compositor-input)
    shell_pid="$(pgrep -u "$SESSION_USER" -x gnome-shell | head -1 || true)"
    if [ -z "$shell_pid" ]; then
      echo "no gnome-shell process for $SESSION_USER" >&2
      exit 1
    fi
    echo "gnome-shell pid $shell_pid"

    echo '--- input devices gnome-shell holds open ---'
    found=0
    for fd in "/proc/$shell_pid/fd"/*; do
      [ -e "$fd" ] || continue
      target="$(readlink "$fd" 2>/dev/null || true)"
      case "$target" in
        /dev/input/*) printf '  %s -> %s\n' "$(basename "$fd")" "$target"; found=1 ;;
      esac
    done
    if [ "$found" = 0 ]; then
      echo '  NONE — the compositor is not reading any input device.'
      echo '  That is the answer: keys reach the kernel and nobody is listening.'
    fi

    # logind hands input devices to the active session through TakeDevice, so
    # if the compositor has none, its view of the seat is where to look next.
    #
    # This used to print `seat-status | sed -n '1,25p'`, which truncated the
    # device tree before any input device appeared -- the first 25 lines are
    # the DRM card, the optical drive, i2c and the power button. The one
    # question being asked, "is the KEYBOARD on this seat", was exactly what
    # got cut off. Filtered to input devices instead of arbitrarily truncated.
    echo '--- what logind thinks the seat has (input devices only) ---'
    if seat="$(loginctl seat-status seat0 2>/dev/null)"; then
      printf '%s\n' "$seat" | sed -n '1,3p'
      inputs="$(printf '%s\n' "$seat" | grep -E '/input/input[0-9]+$' || true)"
      if [ -n "$inputs" ]; then
        printf '%s\n' "$inputs" | sed 's/^/  /'
      else
        echo '  NONE — seat0 has no input devices assigned.'
      fi

      # The keyboard specifically. i8042 is the AT controller the emulated
      # "AT Translated Set 2 keyboard" hangs off; if the compositor reads a
      # device logind has not put on this seat, that mismatch is the lead.
      if printf '%s\n' "$seat" | grep -q 'i8042'; then
        echo '  -> the AT keyboard IS on seat0'
      else
        echo '  -> the AT keyboard is NOT on seat0 (i8042 absent from the tree)'
      fi
    else
      echo '(seat-status unavailable)'
    fi

    # IS THE SESSION ACTIVE?
    #
    # This is the question the rest of the evidence now points at. Everything
    # else is green: the keyboard is on seat0, the kernel receives clean
    # LEFTMETA press and release on event1, and gnome-shell holds event1 open.
    # Yet pressing Super alone leaves the framebuffer byte-identical -- and
    # Super alone is GNOME's OWN binding for the Activities overview, nothing
    # to do with our portal shortcut. So the compositor is inert to this input
    # generally, not failing to route one shortcut.
    #
    # logind pauses a session's input devices when the session is not active,
    # and a paused device keeps its file descriptor open -- the fd is how the
    # resume is delivered. So "gnome-shell holds event1" is entirely consistent
    # with gnome-shell receiving nothing from it, and Active= is what tells the
    # two apart.
    echo '--- is the session active? (paused devices keep their fds) ---'
    sid="$(loginctl list-sessions --no-legend 2>/dev/null | awk -v u="$SESSION_USER" '$3 == u {print $1; exit}')"
    if [ -n "$sid" ]; then
      loginctl show-session "$sid" \
        -p Id -p User -p Name -p Seat -p Type -p Class -p State -p Active -p Remote \
        2>/dev/null | sed 's/^/  /'
    else
      echo "  no logind session found for $SESSION_USER"
      loginctl list-sessions --no-legend 2>/dev/null | sed 's/^/  /' || true
    fi
    ;;

  # Does an injected scancode reach the guest KERNEL?
  #
  # This splits the one question left about Spike A. Pressing Super alone
  # changes nothing on screen, and the guest does have a keyboard — the
  # evidence check shows an "AT Translated Set 2 keyboard" with an evdev node,
  # so the earlier guess that corral adds no input device was wrong. That
  # leaves two possibilities that look identical from outside:
  #
  #   a. QEMU never delivers the scancode to the emulated keyboard, or
  #   b. it does, and something above the kernel — mutter, the seat, focus —
  #      discards it.
  #
  # Reading the evdev node decides it. Bytes arriving while the host injects a
  # key means the kernel got the event and (b) is the answer; silence means (a),
  # and it is corral's or QEMU's to fix rather than ours.
  #
  # The node is resolved from /proc/bus/input/devices rather than hard-coded to
  # event1: it is event1 today, and a hard-coded node that silently moves would
  # report "no input" for a keyboard that is working perfectly.
  keyboard-capture-start)
    node="$(awk '
      /^N: Name=/ { name=$0 }
      /^H: Handlers=/ { handlers=$0 }
      /^B: EV=/ {
        if (name ~ /[Kk]eyboard/ && handlers ~ /event[0-9]+/) {
          match(handlers, /event[0-9]+/)
          print substr(handlers, RSTART, RLENGTH)
        }
        name=""; handlers=""
      }' /proc/bus/input/devices | head -1)"
    if [ -z "$node" ]; then
      echo "no keyboard evdev node found; /proc/bus/input/devices follows" >&2
      cat /proc/bus/input/devices >&2
      exit 1
    fi
    echo "capturing from /dev/input/$node"
    rm -f "$KBD_CAP" "$KBD_CAP_DONE"
    # timeout, not a kill later: the capture must end on its own even if the
    # collector never runs, or a failed run leaves a cat holding the device.
    setsid bash -c '
      timeout 25 cat "/dev/input/$1" > "$2" 2>/dev/null
      echo done > "$3"
    ' _ "$node" "$KBD_CAP" "$KBD_CAP_DONE" < /dev/null > /dev/null 2>&1 &
    # A moment for the redirect to actually open the device, so a key pressed
    # immediately afterwards is not injected into a capture that has not started.
    sleep 2
    ;;

  # Read back what the kernel saw. Run after the host has injected the key.
  keyboard-capture-report)
    wait_for "the keyboard capture to finish" 40 test -f "$KBD_CAP_DONE"
    bytes=$(wc -c < "$KBD_CAP" 2>/dev/null || echo 0)
    echo "captured $bytes bytes of input events while the key was injected"

    # Decode them rather than trusting the byte count. 288 bytes is exactly
    # what two press/release pairs should produce, which is suggestive and is
    # not proof: autorepeat, a stray mouse event or a jittering power button
    # would also make bytes. Printing the keycodes says what actually arrived.
    #
    # struct input_event on 64-bit is two __kernel_ulong_t of timeval, then
    # __u16 type, __u16 code, __s32 value — 24 bytes, "qqHHi".
    python3 - "$KBD_CAP" <<'PY' || echo '(decode failed; the byte count above stands)' 
import struct, sys

SIZE = struct.calcsize("qqHHi")
EV_KEY = 0x01
NAMES = {1: "ESC", 57: "SPACE", 125: "LEFTMETA", 126: "RIGHTMETA"}
ACTION = {0: "release", 1: "press", 2: "autorepeat"}

data = open(sys.argv[1], "rb").read()
if len(data) % SIZE:
    print(f"  warning: {len(data)} bytes is not a whole number of {SIZE}-byte events")

keys = []
for off in range(0, len(data) - SIZE + 1, SIZE):
    _sec, _usec, etype, code, value = struct.unpack("qqHHi", data[off : off + SIZE])
    if etype == EV_KEY:
        keys.append((code, value))
        print(f"  EV_KEY code={code} ({NAMES.get(code, 'other')}) "
              f"value={value} ({ACTION.get(value, '?')})")

print(f"  {len(data) // SIZE} events total, {len(keys)} of them key events")
if not keys:
    print("  NOTE: bytes arrived but none were key events — the count alone "
          "would have been misleading")
PY

    if [ "$bytes" -gt 0 ]; then
      echo "VERDICT: the scancode REACHED the guest kernel — the loss is above it"
    else
      echo "VERDICT: NOTHING reached the guest kernel — QEMU never delivered it"
    fi
    ;;

  # Evidence for Spike A, gathered before the spike runs so that a hang has
  # something to be read against. Nothing here asserts: each line is a fact the
  # report needs in order to be interpretable, and a missing fact is itself
  # worth seeing.
  #
  # Two questions, and they fail in ways that look identical from the client:
  #
  #   1. Did the pre-seed land? If the system dconf database did not compile,
  #      or the profile does not reference it, the grant is simply absent and
  #      BindShortcuts hangs at the consent dialog exactly as it did before.
  #      Read as the session user, through the same dconf profile GNOME uses —
  #      reading the keyfile in /etc would only prove we wrote a file.
  #   2. Does Super+Space already belong to something else? GNOME binds it to
  #      the input-source switcher by default. A collision does not stop the
  #      bind; it means the compositor may route the key elsewhere, so
  #      "activated: false" would mean "someone else got the key" rather than
  #      "the portal does not deliver". Those are different answers and the
  #      spike cannot tell them apart on its own.
  spike-a-evidence)
    u="$(uid)"
    as_session() {
      runuser -u "$SESSION_USER" -- env \
        XDG_RUNTIME_DIR="/run/user/$u" \
        DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/$u/bus" \
        "$@"
    }

    echo '--- dconf profile ---'
    cat /etc/dconf/profile/user 2>/dev/null || echo '(no profile — the seed cannot be visible)'
    echo '--- compiled system database ---'
    ls -l /etc/dconf/db/compass 2>/dev/null || echo '(not compiled — dconf update did not run or found nothing)'

    echo '--- the seeded grant, as the session user sees it ---'
    as_session dconf read \
      "/org/gnome/settings-daemon/global-shortcuts/$APP/shortcuts" \
      || echo '(unreadable)'
    as_session dconf read \
      /org/gnome/settings-daemon/global-shortcuts/applications || true

    # Is there a keyboard at all? Measured after the first run where an
    # injected key changed nothing: corral builds its QEMU command line with
    # `-vga virtio -display none` and adds virtio net and rng devices, but no
    # input device. On x86 the default machine still provides a PS/2 controller,
    # so a keyboard *should* be here — "should" being exactly the word that
    # earned this check. If /proc/bus/input/devices lists no keyboard, the
    # scancodes have nowhere to arrive and no amount of portal work matters.
    echo '--- input devices the kernel knows about ---'
    grep -iE '^[NHB]: (Name|Handlers|EV)' /proc/bus/input/devices 2>/dev/null \
      || echo '(no /proc/bus/input/devices — no input subsystem at all)'
    echo '--- and what libinput sees, if it is installed ---'
    as_session libinput list-devices 2>/dev/null | grep -iE 'Device:|Capabilities:' \
      || echo '(libinput not available in the session; the kernel list above is the evidence)'

    echo '--- which GNOME this is (Phase 1 gates on 50 and 51) ---'
    as_session gnome-shell --version 2>/dev/null || echo '(gnome-shell --version unavailable)'
    grep -E '^(NAME|VERSION|VERSION_ID)=' /etc/os-release 2>/dev/null || true

    echo '--- who else wants Super+Space ---'
    # Not exhaustive and not meant to be: these are the two schemas whose
    # defaults actually collide on this combination. Anything else that claims
    # it will show up as a keypress that never arrives, which is why the raw
    # spike report stays the primary evidence.
    as_session gsettings get org.gnome.desktop.wm.keybindings switch-input-source 2>/dev/null || true
    as_session gsettings get org.gnome.desktop.wm.keybindings switch-input-source-backward 2>/dev/null || true
    as_session gsettings get org.gnome.shell.keybindings toggle-overview 2>/dev/null || true
    ;;

  # Open the launcher in the session and leave it open.
  #
  # This is the first check that exercises the product rather than the platform
  # under it. Everything above answers "can compass run here"; this answers
  # "does compass draw a launcher here", which is the question the tier was
  # built for and could not ask until there was a launcher to open.
  #
  # Same setsid + sentinel shape as spike-a-start, and for the same reasons:
  # without detaching and redirecting all three fds, ssh waits for the channel
  # and the check never returns; and a sentinel file carries the exit status
  # without the `pgrep -f` self-match that made an earlier waiter here time out
  # every single time.
  #
  # Unlike the spike, this process is meant to *stay running* — the sentinel
  # appearing at all is the failure, not the success.
  launcher-start)
    u="$(uid)"
    : > "$UI_ERR"
    rm -f "$UI_DONE"

    # §8.5's last unmeasured row is "Cold start to first frame < 120 ms", and it
    # had no measurement anywhere. This is the closest thing the guest can
    # honestly produce, and the gap between it and the SLA is stated below
    # rather than glossed.
    start_ms="$(date +%s%3N)"

    # RUST_LOG and the wgpu/winit knobs are set because the first run of this
    # job produced a launcher that started, stayed alive, exited nothing, and
    # printed *not one line* — while never putting a window on screen. A silent
    # failure is the worst kind to debug from a 25-minute VM job, so the next
    # run is made to talk.
    #
    # WGPU_BACKEND is deliberately not pinned to a value: naming one would
    # decide the answer instead of measuring it. What is wanted is wgpu's own
    # account of which adapters it found under llvmpipe, which `info` gives.
    setsid bash -c '
      runuser -u "$1" -- env \
        XDG_RUNTIME_DIR="/run/user/$2" \
        DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/$2/bus" \
        WAYLAND_DISPLAY="$3" \
        XDG_SESSION_TYPE=wayland \
        RUST_LOG="info,wgpu=debug,wgpu_hal=debug,iced_wgpu=debug,winit=debug,\
sctk_adwaita=debug,smithay_client_toolkit=debug,wayland_client=debug,calloop=debug" \
        RUST_BACKTRACE=1 \
        flatpak run --installation="$4" "$5" ui \
        > "$6" 2>&1
      echo "$?" > "$7"
    ' _ "$SESSION_USER" "$u" "$(wayland_display)" "$INSTALLATION" "$APP" \
      "$UI_ERR" "$UI_DONE" \
      < /dev/null >> "$UI_ERR" 2>&1 &

    # Nothing inside the guest can observe "has painted" — ADR-0010 says so and
    # it is still true: the framebuffer's only observer is corral, on the far
    # side of QEMU. So what is waited on here is the weaker but real thing, the
    # app process existing under the session user. A launcher that dies on
    # startup — the likeliest failure, and one that would otherwise surface as
    # an inscrutable unchanged frame — fails here instead, with its own output
    # attached. The host screenshot is what closes the gap between "the process
    # is alive" and "a launcher is on screen"; neither half is sufficient.
    #
    # `pgrep -u ... -x` matches the process *name*, deliberately, not `-f`
    # against the command line: an `-f` pattern distinctive enough to find this
    # process is also present in the command line of the shell doing the
    # matching, so the predicate matches itself and is true before the launcher
    # has done anything at all. That exact bug cost an earlier waiter here 180
    # seconds a run, and it presents as a timeout rather than as a mistake.
    wait_for "the launcher process to appear, or exit" 90 launcher_appeared
    spawned_ms="$(date +%s%3N)"

    # And then wait for it to be READY, which is not the same thing and cost
    # three runs and a wrong conclusion to learn.
    #
    # The process existing says nothing about whether anything is on screen.
    # Under llvmpipe this launcher takes seconds to first paint and the spread
    # is wide: one run had wgpu initialising 2.4s after start, another had not
    # touched wgpu 8.1s in. Screenshotting at process-appear caught the second
    # kind twice and produced "the launcher does not draw", which was wrong.
    #
    # ADR-0010's own rule is to key off a state and never a duration, and this
    # check was breaking it. `Adapter AdapterInfo` in the launcher's own log is
    # a real state: wgpu only reports a chosen adapter once it has a surface to
    # render to. The settle after it is slack after a state, the same shape and
    # the same justification as wait-graphical.sh.
    wait_for "the renderer to choose an adapter, or the launcher to exit" 150 \
      bash -c 'grep -q "Adapter AdapterInfo" "$1" || [ -f "$2" ]' \
      _ "$UI_ERR" "$UI_DONE"
    ready_ms="$(date +%s%3N)"

    # WHAT THIS NUMBER IS, AND WHAT §8.5 ASKED FOR.
    #
    # The SLA is "cold start to first frame". Nothing inside the guest can
    # observe a frame — ADR-0010 settles that, and it is why the paint gate
    # lives on the host with corral's screenshots. So this measures the nearest
    # state the guest CAN see: `Adapter AdapterInfo`, which wgpu emits only once
    # it has a surface to render to. First paint follows shortly after.
    #
    # It is REPORTED, NOT GATED, for the same reason §11.2 reports RSS rather
    # than gating it. Under llvmpipe on a QEMU guest this is software rendering
    # on emulated hardware, and the spread is enormous — ADR-0010 records 2.4 s
    # to wgpu in one run against not-yet at 8.1 s in another. A 120 ms budget
    # measured here would be measuring the VM, not the launcher, and gating on
    # it would make the tier red for reasons that have nothing to do with the
    # code.
    #
    # The split matters too: spawn cost is Flatpak and process start, render
    # cost is wgpu bringing up a software adapter. Only the second is what the
    # SLA is about, and only the first would improve on real hardware.
    echo "cold start: $((spawned_ms - start_ms)) ms to process, \
$((ready_ms - spawned_ms)) ms process to renderer-ready, \
$((ready_ms - start_ms)) ms total (llvmpipe, reported not gated — see §8.5)"

    sleep 3

    if [ -f "$UI_DONE" ]; then
      echo "the launcher exited $(cat "$UI_DONE") instead of staying open; its output follows" >&2
      cat "$UI_ERR" >&2
      exit 1
    fi
    echo "the launcher is running; output so far:"
    cat "$UI_ERR"
    ;;

  # What is the launcher actually blocked on?
  #
  # The instrumented run answered one question and posed a sharper one: the log
  # ends 200 ms in, at an `sctk_adwaita` XDG-Settings-Portal timeout inside
  # `create_window`, and then nothing — no wgpu lines at all, though wgpu,
  # wgpu_hal and iced_wgpu were all at debug. So execution never reaches wgpu,
  # and the process sits there alive for the rest of the run.
  #
  # Logs cannot say where it stopped, because the stuck code is not logging.
  # The kernel can. /proc/PID/wchan and /proc/PID/syscall name the syscall a
  # thread is parked in, and the open fds say which socket it is parked on —
  # a Wayland socket and a D-Bus socket look nothing alike here. No debugger
  # needed, nothing to install, and it works on a stripped image.
  #
  # Reading the host's /proc for a Flatpak process is fine: bwrap namespaces
  # the guest's view, not root's.
  launcher-diagnose)
    pid="$(launcher_pid || true)"
    if [ -z "$pid" ]; then
      echo "no vicinae process to diagnose"
      exit 0
    fi
    echo "pid $pid"
    echo "--- state ---"
    grep -E '^(State|Threads|SigBlk):' "/proc/$pid/status" 2>/dev/null || true
    echo "--- per-thread: what each one is parked in ---"
    # The main thread is the interesting one, but a hang in a worker looks
    # identical from outside, so all of them are printed.
    for t in /proc/"$pid"/task/*; do
      tid="$(basename "$t")"
      printf '  tid %s  state=%s  wchan=%s\n' \
        "$tid" \
        "$(awk '/^State:/{print $2}' "$t/status" 2>/dev/null)" \
        "$(cat "$t/wchan" 2>/dev/null || echo '?')"
      printf '    syscall: %s\n' "$(cat "$t/syscall" 2>/dev/null || echo '?')"
    done
    echo "--- sockets it holds open ---"
    # readlink over a glob rather than `ls -l | grep`: shellcheck SC2010 is
    # right that parsing ls breaks on odd names, and /proc/PID/fd is exactly
    # where symlink targets get interesting.
    found=0
    for fd in "/proc/$pid/fd"/*; do
      [ -e "$fd" ] || continue
      target="$(readlink "$fd" 2>/dev/null || true)"
      case "$target" in
        socket:*|*wayland*|*dbus*|*bus)
          printf '  %s -> %s\n' "$(basename "$fd")" "$target"; found=1 ;;
      esac
    done
    [ "$found" = 1 ] || echo '(no sockets listed)'
    echo "--- and which of those the kernel can name ---"
    # The first version of this printed "(ss unavailable or no match)", which
    # conflates a missing tool with a tool that found nothing — and those want
    # different fixes. It reported exactly that, uselessly. Say which.
    if ! command -v ss >/dev/null 2>&1; then
      echo '(ss is not in this image; falling back to /proc/net/unix below)'
    elif ! ss -x -p 2>/dev/null | grep -F "pid=$pid"; then
      echo '(ss ran and matched no socket for this pid)'
    fi

    # The fallback, which needs no tools at all. Note its limit up front: a
    # *connected* AF_UNIX socket has an empty path column, so this names the
    # listening sockets and leaves client ends as "(unnamed)". Verified on this
    # machine before shipping rather than discovered in the guest.
    echo '--- socket inodes against /proc/net/unix ---'
    for fd in "/proc/$pid/fd"/*; do
      [ -e "$fd" ] || continue
      target="$(readlink "$fd" 2>/dev/null || true)"
      case "$target" in
        socket:*)
          ino="${target#socket:[}"; ino="${ino%]}"
          path="$(awk -v i="$ino" '$7==i {print ($8==""?"(unnamed — connected end)":$8)}' \
                  /proc/net/unix 2>/dev/null)"
          printf '  fd %s inode %s -> %s\n' \
            "$(basename "$fd")" "$ino" "${path:-(not listed)}" ;;
      esac
    done
    ;;

  # Phase 1's gate says "idle RSS < 30 MB". Nobody had measured it, because
  # until the launcher drew there was nothing to measure.
  #
  # Read after the window is up and the run has gone quiet, which is what
  # "idle" means here: the renderer has an adapter, the first frame is done,
  # and nothing is being typed. RSS is taken from the host's /proc — a Flatpak
  # process is an ordinary process to the kernel, and VmRSS is the number the
  # gate is phrased in.
  #
  # Reported, not gated. A number measured once is not a budget, and a memory
  # gate set from a single sample on a software-rendered VM would be the
  # deviation mistake again in a different costume. It goes in the log so the
  # gate can be set from a distribution later.
  launcher-rss)
    pid="$(launcher_pid || true)"
    if [ -z "$pid" ]; then
      echo "no launcher process to measure" >&2
      exit 1
    fi
    rss_kb="$(awk '/^VmRSS:/{print $2}' "/proc/$pid/status" 2>/dev/null || echo 0)"
    hwm_kb="$(awk '/^VmHWM:/{print $2}' "/proc/$pid/status" 2>/dev/null || echo 0)"
    printf 'launcher pid %s: VmRSS %s kB (%s MB), peak VmHWM %s kB (%s MB)\n' \
      "$pid" "$rss_kb" "$((rss_kb / 1024))" "$hwm_kb" "$((hwm_kb / 1024))"
    # THE ENGINE IS MEASURED TOO, AND IT IS THE MORE HONEST NUMBER.
    #
    # Phase 1's gate says "idle RSS < 30 MB" without naming a process, and this
    # check used to answer it with the window's figure alone -- ~135 MB, read as
    # our code being five times over budget. Most of that is wgpu's software
    # renderer: under llvmpipe the GPU stack lives in this process's RSS, and on
    # hardware it does not.
    #
    # The engine is the part that is actually resident. It holds the index and
    # serves IPC, it runs whether or not a window is open, and it draws nothing,
    # so its RSS is comparable across a VM and a real machine. Measured on an
    # ordinary x86-64 container outside any VM it idles at 6.2 MB.
    #
    # Both are printed, labelled, and neither is gated -- the numbers decide the
    # threshold, not the other way round (ADR-0010).
    engine_pid="$(head -1 "$ENGINE_PIDS" 2>/dev/null || true)"
    if [ -n "$engine_pid" ] && [ -r "/proc/$engine_pid/status" ]; then
      erss_kb="$(awk '/^VmRSS:/{print $2}' "/proc/$engine_pid/status" 2>/dev/null || echo 0)"
      printf 'engine   pid %s: VmRSS %s kB (%s MB) -- resident, draws nothing\n' \
        "$engine_pid" "$erss_kb" "$((erss_kb / 1024))"
    else
      echo 'engine: no pid recorded, so only the window could be measured'
    fi

    printf 'Phase 1 gate is "idle RSS < 30 MB".\n'
    printf '  engine   %s MB — %s\n' "$((${erss_kb:-0} / 1024))" \
      "$( [ "$((${erss_kb:-0} / 1024))" -lt 30 ] && echo 'under' || echo 'OVER' )"
    printf '  launcher %s MB — %s\n' "$((rss_kb / 1024))" \
      "$( [ "$((rss_kb / 1024))" -lt 30 ] && echo 'under' || echo 'OVER' )"
    # Under llvmpipe the renderer keeps its own buffers, so a VM number is not
    # a hardware number. Said here so nobody reads it as one.
    echo 'note: the launcher figure includes wgpu under software rendering, so it is'
    echo '      an upper bound rather than the shipping figure; the engine figure is not.'
    ;;

  # Harvest a real desktop-entry corpus from this Bluefin box.
  #
  # Phase 1's gate wants ~500 real entries for Suite 0 ranking parity and there
  # are 27 (§11.2). The harvester's own header asks for "a real desktop —
  # ideally a Bluefin box", and that is exactly what this VM is: a full GNOME
  # application set on the target platform, booted fresh every run.
  #
  # It writes into a scratch directory rather than the repo's corpus path,
  # because nothing here should look like it commits to the repository. The
  # tarball goes out with the artifacts and a person decides what to keep — a
  # corpus is test input that shapes every ranking assertion, and it should not
  # grow by a job quietly appending to it.
  # Are the machine's Flatpak applications in the index? (#105)
  #
  # A Flatpak export is a SYMLINK, not a file. flatpak-dir.c builds it with the
  # prefix `../app/<id>/current/active/export`, so the entry a launcher reads
  # lives in the deploy tree and the exports directory holds only pointers into
  # it. Our sandbox granted the exports directories and not the deploy trees,
  # which meant every Flatpak on the machine was a dangling link inside it --
  # invisible, with no error anywhere, because the scan still lists the name and
  # the read fails one step later.
  #
  # Unit tests pin the shape of that (compass-xdg) and the manifest invariant
  # (xdg_dirs), but only a real session can answer whether the sandbox actually
  # resolves them. That is what this is: two applications installed by flatpak
  # itself, one in the system root and one in the user's, queried through the
  # running engine from inside the Flatpak.
  #
  # The display name is read from the export rather than written here, so the
  # check does not break when an upstream renames its application; the app id is
  # what is asserted, because that is what the index keys on.
  flatpak-apps)
    status=0
    for spec in \
      "system:/var/lib/flatpak/exports/share/applications" \
      "user:/var/home/$SESSION_USER/.local/share/flatpak/exports/share/applications"
    do
      root="${spec%%:*}"
      dir="${spec#*:}"

      if [ ! -d "$dir" ]; then
        echo "FAIL: $root: $dir does not exist, so the image never installed a probe app" >&2
        status=1
        continue
      fi

      found=0
      for entry in "$dir"/*.desktop; do
        # `-e` follows the link, so a DANGLING export fails it and the glob's
        # own literal fallback fails it too. Testing `-L` as well keeps the two
        # apart: a dangling link here is a broken image and must be reported as
        # one, not skipped into "no exports at all", which is what the first
        # draft of this did.
        [ -e "$entry" ] || [ -L "$entry" ] || continue
        found=1
        app_id="$(basename "$entry" .desktop)"

        if [ ! -L "$entry" ]; then
          echo "note: $root: $app_id is not a symlink; flatpak's export layout has changed" >&2
        elif [ ! -e "$entry" ]; then
          # Note this is the view from OUTSIDE the sandbox, as root. #105 is a
          # link that dangles only *inside* it, which no check here can see --
          # the query below is what answers that. A link broken out here is a
          # different and worse thing: the image itself is wrong.
          echo "FAIL: $root: $app_id is a dangling symlink on the host: $(readlink "$entry")" >&2
          echo "  the image installed it and then lost the deploy tree; this is not #105" >&2
          status=1
          continue
        fi

        name="$(sed -n 's/^Name=//p' "$entry" | head -1)"
        if [ -z "$name" ]; then
          echo "FAIL: $root: no Name= in $entry" >&2
          status=1
          continue
        fi

        echo "--- $root: $app_id ($name) ---"
        if compass_cli query --json "$name" \
             > /tmp/flatpak-query.json 2>/tmp/flatpak-query.err
        then
          APP_ID="$app_id" NAME="$name" ROOT="$root" python3 - <<'PY' || status=1
import json, os, sys

app_id = os.environ["APP_ID"]
name = os.environ["NAME"]
root = os.environ["ROOT"]
hits = json.load(open("/tmp/flatpak-query.json"))
for hit in hits[:5]:
    print(f"  {hit['score']:3} {hit['id']}  {hit['title']}")
if any(hit["id"] in (f"{app_id}.desktop", app_id) for hit in hits):
    print(f"  ok: the {root} Flatpak {app_id} is in the index")
    sys.exit(0)
sys.exit(
    f"FAIL: querying {name!r} did not return the {root} Flatpak {app_id}.\n"
    "  Its exported .desktop is a symlink into the deploy tree; if the sandbox "
    "cannot read that tree the entry is invisible and every Flatpak on the "
    "machine disappears from search (#105).\n"
    "  Check --filesystem=/var/lib/flatpak/app:ro and "
    "--filesystem=xdg-data/flatpak/app:ro in the manifest."
)
PY
        else
          echo "FAIL: $root: the query itself failed" >&2
          cat /tmp/flatpak-query.err >&2
          status=1
        fi
      done

      if [ "$found" -eq 0 ]; then
        echo "FAIL: $root: no .desktop exports in $dir; the image installed no probe app" >&2
        status=1
      fi
    done
    exit "$status"
    ;;

  harvest-corpus)
    scratch=/tmp/compass-corpus
    rm -rf "$scratch"; mkdir -p "$scratch"
    /usr/libexec/compass-vmtest/harvest-desktop-corpus.sh --out "$scratch" || true

    count=$(find "$scratch" -type f -name '*.desktop' | wc -l)
    echo "harvested $count desktop entries from this image"
    if [ "$count" -eq 0 ]; then
      echo 'no entries harvested — the roots the script knows about are all empty' >&2
      exit 1
    fi

    # Deterministic ordering and no timestamps, so re-running produces the same
    # bytes and a diff of two harvests is a diff of the application set.
    tar --sort=name --mtime='UTC 1970-01-01' --owner=0 --group=0 --numeric-owner \
      -czf "$KBD_CAP_DIR/corpus.tar.gz" -C "$scratch" . 2>/dev/null \
      || tar -czf "$KBD_CAP_DIR/corpus.tar.gz" -C "$scratch" .
    echo "wrote $KBD_CAP_DIR/corpus.tar.gz ($(wc -c < "$KBD_CAP_DIR/corpus.tar.gz") bytes)"

    # A sample in the log, so the artifact is not the only way to see what came
    # out and a wrong-looking harvest is obvious in the run itself.
    echo '--- first 15 entries ---'
    find "$scratch" -type f -name '*.desktop' -printf '  %f\n' | sort | head -15
    ;;

  # Assert the launcher is still up, and say what it printed.
  #
  # Run after the host has screenshotted and typed at it. A launcher that
  # The control this job should have had from the start: can ANY client draw
  # in this session?
  #
  # Everything measured so far says our launcher does not put a window on
  # screen. None of it distinguishes that from *nothing* putting a window on
  # screen — a session where no client can render at all would produce exactly
  # the same evidence, and would exonerate the launcher entirely. The desktop
  # itself painting (deviation 0.1564) does not settle it: that is GNOME Shell
  # compositing its own furniture, not a client surface.
  #
  # So: start a stock GNOME application and let the host screenshot. If it
  # draws and ours does not, the fault is ours. If neither draws, the fault is
  # the session, and every conclusion about the launcher is void.
  #
  # The application is chosen at runtime from what the image actually has,
  # rather than guessed at here: a hard-coded name that is absent reads as
  # "the control failed" when it means "the control never ran".
  control-app-start)
    u="$(uid)"
    : > "$CONTROL_ERR"
    rm -f "$CONTROL_DONE"

    app=""
    for candidate in gnome-text-editor nautilus gnome-calculator ptyxis gnome-terminal gedit; do
      if command -v "$candidate" >/dev/null 2>&1; then app="$candidate"; break; fi
    done
    if [ -z "$app" ]; then
      echo "no stock GNOME application found to use as a control" >&2
      echo "tried: gnome-text-editor nautilus gnome-calculator ptyxis gnome-terminal gedit" >&2
      exit 1
    fi
    echo "control application: $app"

    setsid bash -c '
      runuser -u "$1" -- env \
        XDG_RUNTIME_DIR="/run/user/$2" \
        DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/$2/bus" \
        WAYLAND_DISPLAY="$3" \
        XDG_SESSION_TYPE=wayland \
        "$4" > "$5" 2>&1
      echo "$?" > "$6"
    ' _ "$SESSION_USER" "$u" "$(wayland_display)" "$app" \
      "$CONTROL_ERR" "$CONTROL_DONE" \
      < /dev/null >> "$CONTROL_ERR" 2>&1 &

    # NOT pgrep. `-f "$app"` would match the shell evaluating it, because the
    # app's name is in that shell's own command line — the same reflexivity
    # that cost Spike A's collector 180s a run, and which I wrote again here
    # before catching it. `-x` is no escape either: comm is truncated to 15
    # characters, so `gnome-text-editor` is `gnome-text-edit` and an exact
    # match silently never fires.
    #
    # Comparing /proc/PID/exe has neither problem: it is the resolved binary,
    # not a string anyone typed, and it is not truncated. `exe_running` is a
    # function, which works because wait_for invokes "$@" in this same shell.
    wait_for "the control application to appear, or exit" 90 \
      exe_running "$app" "$CONTROL_DONE"

    if [ -f "$CONTROL_DONE" ]; then
      echo "the control application exited $(cat "$CONTROL_DONE"); its output follows"
      cat "$CONTROL_ERR"
    else
      echo "the control application is running"
      cat "$CONTROL_ERR"
    fi
    ;;

  # crashed on the first keystroke is a real bug and would otherwise be visible
  # only as two screenshots that happen to look similar.
  launcher-status)
    if [ -f "$UI_DONE" ]; then
      echo "the launcher exited $(cat "$UI_DONE")" >&2
      cat "$UI_ERR" >&2
      exit 1
    fi
    echo "the launcher is still running"
    cat "$UI_ERR"
    ;;

  # Start the engine, so the launcher has something to attach to.
  #
  # ADR-0015 made the launcher window resident and driven: `vicinae ui` connects
  # to `vicinae serve` and waits to be told to show. So the engine has to be up
  # BEFORE launcher-start, or the launcher comes up undriven and every summon
  # below is refused -- correctly, and confusingly.
  #
  # Inside the same Flatpak and the same session as the launcher, because the
  # socket lives under $XDG_RUNTIME_DIR and a daemon in a different runtime dir
  # is a daemon the launcher cannot find.
  #
  # WITH `--no-hotkey`, AND THAT WAS MEASURED RATHER THAN ASSUMED. Binding the
  # GlobalShortcuts portal makes GNOME ask the user for permission, and step 0d
  # measured what that puts on screen: 1.62% of pixels in a 496x532 box. That
  # is correct product behaviour and it is Spike A's job to exercise it -- but
  # inside THIS job it sits between the frames of a gate about the launcher,
  # and moving the control after the engine was not enough, because the
  # launcher window then interacts with it. Two runs failed identically at
  # 962x603 before the flag existed.
  #
  # The flag is not a test hook: a user whose compositor binds a key to
  # `vicinae toggle` should not be asked to grant one they will not use.
  engine-start)
    u="$(uid)"
    : > "$ENGINE_ERR"
    rm -f "$ENGINE_DONE"

    setsid bash -c '
      runuser -u "$1" -- env \
        XDG_RUNTIME_DIR="/run/user/$2" \
        DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/$2/bus" \
        WAYLAND_DISPLAY="$3" \
        XDG_SESSION_TYPE=wayland \
        RUST_LOG=info \
        RUST_BACKTRACE=1 \
        flatpak run --installation="$4" "$5" serve --no-hotkey \
        > "$6" 2>&1
      echo "$?" > "$7"
    ' _ "$SESSION_USER" "$u" "$(wayland_display)" "$INSTALLATION" "$APP" \
      "$ENGINE_ERR" "$ENGINE_DONE" \
      < /dev/null >> "$ENGINE_ERR" 2>&1 &

    # Waited on by asking it, not by looking for its process. A `serve` that is
    # alive but has not yet bound its socket would pass a pgrep and fail every
    # summon after it -- and the engine indexes the machine's applications
    # before it listens, which under llvmpipe is not instant. `ping` answers
    # only once the socket is up.
    # A bare `wait_for` here says only "timed out", which is the same message
    # for an engine that is still indexing, an engine that crashed, and a
    # client invocation that could never have worked. The last of those is
    # exactly what happened once, and the 120 s of silence is what made it
    # expensive. So the timeout now says what `ping` actually returns.
    if ! wait_for "the engine to answer a ping, or exit" 120 engine_ready; then
      echo "--- what the client actually says ---" >&2
      compass_cli ping >&2 2>&1 || true
      echo "--- the engine's own output ---" >&2
      cat "$ENGINE_ERR" >&2
      exit 1
    fi

    # Recorded before the launcher starts, so `launcher_pid` can tell the two
    # apart. Written even on the failure path below: a half-started engine
    # still leaves a process named `vicinae` around to be mistaken for the
    # launcher.
    pgrep -u "$SESSION_USER" -x vicinae > "$ENGINE_PIDS" 2>/dev/null || : > "$ENGINE_PIDS"
    echo "engine pids: $(tr '\n' ' ' < "$ENGINE_PIDS")"

    if [ -f "$ENGINE_DONE" ]; then
      echo "the engine exited $(cat "$ENGINE_DONE") instead of listening; its output follows" >&2
      cat "$ENGINE_ERR" >&2
      exit 1
    fi
    echo "the engine is listening; output so far:"
    cat "$ENGINE_ERR"
    ;;

  # Is the launcher window actually attached to the engine?
  #
  # The distinguishing check, and the reason the summon verbs below can mean
  # anything. `serve` refuses show/hide/toggle when no window has attached, so
  # a `toggle` that SUCCEEDS is proof that the whole chain exists: CLI, socket,
  # engine, window link, and a window that answered. A `toggle` that fails with
  # "no launcher window is connected" says precisely which link is missing.
  #
  # This is what the tier could never assert before: every earlier check could
  # only see a process, never a connection.
  window-attached)
    if ! out="$(compass_cli toggle 2>&1)"; then
      echo "the launcher window is not attached to the engine:" >&2
      echo "$out" >&2
      exit 1
    fi
    # Left hidden by the toggle above; `summon` below puts it back. Said out
    # loud because a reader of the log otherwise sees a window vanish for no
    # stated reason.
    echo "the window answered a toggle (and is now hidden): $out"
    ;;

  # Summon the window and report how long the round trip took.
  #
  # §8.5's "summon to first frame" row. This is NOT that number: nothing inside
  # the guest can observe a frame (ADR-0010), so what is timed here is the
  # round trip -- CLI to engine, engine to window, window's answer back. First
  # paint follows it.
  #
  # It is also inflated by a whole process spawn, because the client is
  # `vicinae toggle` rather than a keypress. On the real path the engine is
  # already running and the portal delivers the activation directly, so this
  # number is an upper bound with a Flatpak launch inside it. Reported, not
  # gated, for the reasons §8.5 gives.
  summon)
    start_ms="$(date +%s%3N)"
    if ! out="$(compass_cli show 2>&1)"; then
      echo "the engine could not show the window:" >&2
      echo "$out" >&2
      exit 1
    fi
    end_ms="$(date +%s%3N)"
    echo "summon round trip: $((end_ms - start_ms)) ms \
(client spawn included; not a frame -- see §8.5)"
    ;;

  # Hide it again, so the host can screenshot the difference.
  dismiss)
    compass_cli hide
    ;;

  # What did the engine say about the hotkey?
  #
  # Recorded, not gated. Whether GNOME grants LOGO+space is the user's decision
  # via a permission dialog, and an unattended session may well be refused --
  # which is a real outcome worth seeing in the log, not a failure of the code.
  hotkey-status)
    grep -E "launcher hotkey|GlobalShortcuts|shortcut" "$ENGINE_ERR" || \
      echo "the engine said nothing about the hotkey"
    ;;

  # Put the session on a bare desktop before anything is measured against it.
  #
  # GNOME Shell opens the Activities overview at login when the session has no
  # windows, and until now the session always had one: the first-run tour. With
  # the tour suppressed the overview is what the tier's "bare desktop" frame
  # actually shows, and the first run after that suppression shows exactly what
  # that costs -- the launcher opened BEHIND the overview and was captured as a
  # scaled thumbnail inside a workspace tile, so the gate that says "a launcher
  # window appeared" was measuring a preview of one.
  #
  # It is dismissed over the session bus rather than by injecting Escape, and
  # the difference is the point: `org.gnome.Shell.OverviewActive` is a readwrite
  # boolean, so this says what it wants and then READS BACK whether it got it. A
  # keystroke can only be sent, and a tier that cannot tell "the overview closed"
  # from "the key went nowhere" is how the wrong conclusion below got written
  # down in the first place.
  #
  # Idempotent by construction: setting it false when it is already false is
  # fine, and the read-back is then trivially true. So this stays correct if a
  # later GNOME stops opening the overview at login, rather than becoming a step
  # that fails because it had nothing to do.
  overview-dismiss)
    u="$(uid)"
    shell_prop() {
      local method="$1"; shift
      runuser -u "$SESSION_USER" -- env \
        XDG_RUNTIME_DIR="/run/user/$u" \
        DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/$u/bus" \
        gdbus call --session \
          --dest org.gnome.Shell --object-path /org/gnome/Shell \
          --method "org.freedesktop.DBus.Properties.$method" \
          org.gnome.Shell OverviewActive "$@"
    }
    overview_closed() { case "$(shell_prop Get 2>/dev/null)" in *false*) return 0 ;; esac; return 1; }

    echo "OverviewActive at login: $(shell_prop Get 2>&1 || echo '(unreadable)')"

    if ! shell_prop Set "<false>" >/dev/null 2>&1; then
      echo "could not set org.gnome.Shell OverviewActive -- is gnome-shell on the session bus?" >&2
      shell_prop Get >&2 2>&1 || true
      exit 1
    fi

    # Read back, rather than trusting the write. The property is set on the
    # shell's side of an async animation, so the value can be false while the
    # overview is still on its way out; the poll covers the first and the sleep
    # covers the second. If the frame is still mid-animation the run does not
    # silently pass on it -- step 0d compares this desktop against the one taken
    # after the engine starts and requires them identical, which a moving
    # overview cannot be.
    if ! wait_for "the overview to report itself closed" 30 overview_closed; then
      echo "OverviewActive is still: $(shell_prop Get 2>&1 || echo '(unreadable)')" >&2
      exit 1
    fi
    sleep 2
    echo "OverviewActive now: $(shell_prop Get 2>&1 || echo '(unreadable)')"
    ;;

  # Did the engine actually find any applications? (#95)
  #
  # THE CHEAPEST GATE IN THIS FILE, AND IT WOULD HAVE SAVED MONTHS.
  #
  # `indexed applications applications=0` was printed by every run of this tier and read by
  # nobody, because an empty index and a working one looked identical on screen: until the search
  # field could be focused at all, both drew "Type to search..." forever. The launcher shipped
  # unable to see a single application on the machine it was running on.
  #
  # A missing line fails as loudly as a zero. "The engine did not say" and "the engine said none"
  # are both answers this check must not treat as success -- a log format change that silently
  # turned this into a no-op is exactly the failure mode the gate exists to prevent.
  engine-index)
    # THE ESCAPES ARE NOT DECORATION, THEY ARE WHY THIS FAILED THE FIRST TIME IT RAN.
    #
    # `tracing_subscriber::fmt` colours its output whether or not the writer is a terminal, so
    # the engine's log file holds
    #
    #     indexed applications ^[[3mapplications^[[0m^[[2m=^[[0m15
    #
    # and a pattern with a literal `applications=` matches nothing. The gate then reported "the
    # engine never reported an application count" while printing the line that contained it --
    # a false negative that read exactly like the real failure it exists to catch.
    #
    # The engine no longer colours a non-terminal, so this is belt and braces. It stays because
    # RUST_LOG and a future writer can both put the escapes back, and a grep in a gate should not
    # be the thing that notices.
    line="$(sed 's/\x1b\[[0-9;]*m//g' "$ENGINE_ERR" \
            | grep -o 'indexed applications applications=[0-9]*' | tail -1 || true)"
    if [ -z "$line" ]; then
      echo "the engine never reported an application count; the log format has probably changed" >&2
      echo "--- what it did say ---" >&2
      cat "$ENGINE_ERR" >&2
      exit 1
    fi

    count="${line##*=}"
    echo "the engine indexed $count applications"
    if [ "$count" -eq 0 ]; then
      echo "the engine indexed ZERO applications: the launcher can see nothing to launch (#95)" >&2
      echo "--- where the doctor looked ---" >&2
      compass_cli doctor 2>&1 | grep -A 20 'xdg.application-dirs' >&2 || true
      exit 1
    fi
    ;;

  *)
    echo "unknown subcommand: $1" >&2
    exit 64
    ;;
esac
