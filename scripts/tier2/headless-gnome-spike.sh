#!/usr/bin/env bash
#
# Tier 2 spike: does a real compositor come up headless in a container? (#119)
#
# PLAN.md §8.8 specifies a tier between "no display, seconds" and "QEMU,
# twenty minutes":
#
#   Tier 2 — per PR, headless GNOME in a container. `gnome-shell --headless
#   --virtual-monitor` in a Fedora 44/45 container gives real Mutter and a real
#   xdg-desktop-portal-gnome without a VM. Reach for this before reaching for
#   QEMU.
#
# It was never built. This script is the spike that answers whether the premise
# holds on a hosted runner, before anything is built on top of it.
#
# WHAT THE FIRST RUN FOUND, AND WHY THIS NOW STARTS MUTTER
#
# It ran `gnome-shell --headless` and got half an answer. Mutter came up
# perfectly — surfaceless renderer, virtual monitor, `wayland-0` listening —
# and then GNOME Shell died in its JavaScript UI layer:
#
#   GNOME Shell-CRITICAL: Gio.IOErrorEnum: Could not connect
#     LoginManagerSystemd@.../misc/loginManager.js:116
#     _init@.../ui/background.js:269
#     _initializeUI@.../ui/main.js:219
#
# Building the background needs a login manager, that needs logind's D-Bus,
# and a plain container has no systemd. So the Shell never claims
# `org.gnome.Shell`, and it never will without systemd as PID 1.
#
# The compositor is what this tier is for. Portal behaviour,
# `xdg-activation-v1` focus semantics and a real Wayland server all live in
# Mutter; none of them live in the Shell. So the spike now asks the question
# the tier actually depends on, and the Shell result is recorded above rather
# than re-run forever.
#
# THE LIMIT THIS IMPOSES, STATED HERE SO NOBODY TRUSTS THE TIER TOO FAR
#
# A Mutter-only session cannot exercise anything Shell-resident — our own
# GNOME Shell extension most of all. Suite 3b still needs a VM, or systemd in
# the container, for those. What this buys is everything at the protocol
# level, per PR, in minutes.
#
# WHAT IT ASSERTS, AND WHY EACH ONE
#
# A spike that only checks "the command did not immediately exit" would pass
# against a shell that crashed a second later, which is the class of check this
# repository keeps having to fix. So it asserts three things, each strictly
# stronger than the last:
#
#   1. mutter is installed and reports a version — rules out a package that
#      silently is not there.
#   2. a Wayland socket appears in XDG_RUNTIME_DIR — the compositor got far
#      enough to listen.
#   3. a real Wayland client CONNECTS to it and reads the global registry.
#      This is the one that matters: a socket file proves a bind() call, not a
#      compositor that will serve anyone. The first run of this spike is the
#      argument for the distinction — the socket appeared and the session was
#      nonetheless unusable.
#   4. the globals that this tier exists to exercise are actually advertised.
#      A compositor serving a registry with no `xdg_activation_v1` in it would
#      pass (3) and still be useless for testing focus semantics.
#
# Nothing here sleeps a plausible number of seconds and hopes. Each wait polls
# for a condition with a deadline, and says which condition it gave up on.

set -euo pipefail

readonly VIRTUAL_MONITOR="${VIRTUAL_MONITOR:-1280x800}"
readonly SHELL_LOG="${SHELL_LOG:-/tmp/gnome-shell.log}"

# The globals this tier would exist to exercise. Asserted by name rather than
# counted, because "the registry had 40 things in it" is not an answer to
# "can we test focus semantics here".
readonly REQUIRED_GLOBALS="wl_compositor wl_seat xdg_wm_base xdg_activation_v1"
readonly STARTUP_TIMEOUT_S="${STARTUP_TIMEOUT_S:-60}"

log() { printf '\n=== %s ===\n' "$*"; }

# Prints the compositor's own log, because that is where the answer is.
#
# This used to live at one call site, after `wait_for`. It never ran: `fail`
# exits, and `wait_for` calls `fail`, so the branch after `wait_for` was
# unreachable. The first real failure of this spike reported "timed out
# waiting for org.gnome.Shell" and nothing else, and the actual cause — a
# missing logind — was only visible by downloading the artifact. Printing from
# `fail` covers every path instead of the one I remembered to handle.
fail() {
  printf 'SPIKE FAILED: %s\n' "$*" >&2
  if [ -s "$SHELL_LOG" ]; then
    printf '\n--- %s ---\n' "$SHELL_LOG" >&2
    cat "$SHELL_LOG" >&2
  fi
  exit 1
}

# Polls `cmd` until it succeeds or the deadline passes.
wait_for() {
  local what="$1" deadline_s="$2"
  shift 2
  local deadline=$((SECONDS + deadline_s))
  while [ "$SECONDS" -lt "$deadline" ]; do
    if "$@" >/dev/null 2>&1; then
      printf 'ok: %s\n' "$what"
      return 0
    fi
    sleep 1
  done
  fail "timed out after ${deadline_s}s waiting for: ${what}"
}

log "what this container has"
# Reported rather than assumed: if the premise fails, the first question is
# always "which GNOME was it".
# Both, because the Shell's absence from the session is a deliberate finding
# (see the header) and the next person should see that it IS installed and
# still not used.
mutter --version || fail "mutter is not installed"
gnome-shell --version || echo "(gnome-shell present but not started — see the header)"
echo "fedora: $(rpm -E %fedora 2>/dev/null || echo unknown)"
echo "mutter: $(rpm -q mutter 2>/dev/null || echo 'not installed')"
echo "portal: $(rpm -q xdg-desktop-portal-gnome 2>/dev/null || echo 'not installed')"

# Checked up front so a missing tool is one clear line rather than a "command
# not found" followed by a `set -u` unbound-variable cascade, which is how the
# first run of this spike reported that dbus-launch lives in dbus-x11.
log "checking the tools this spike needs"
for tool in dbus-launch gdbus mutter wayland-info; do
  command -v "$tool" >/dev/null 2>&1 \
    || fail "${tool} is not installed — the container's package list is wrong, not the premise"
  printf 'have: %s (%s)\n' "$tool" "$(command -v "$tool")"
done

log "starting a session bus"
export XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-/run/user/0}"
mkdir -p "$XDG_RUNTIME_DIR"
chmod 700 "$XDG_RUNTIME_DIR"

eval "$(dbus-launch --sh-syntax)"
export DBUS_SESSION_BUS_ADDRESS
echo "bus: ${DBUS_SESSION_BUS_ADDRESS}"

log "starting mutter --headless --virtual-monitor ${VIRTUAL_MONITOR}"
# Logged to a file rather than discarded: when this fails, the shell's own
# output is the only thing that says why, and a spike that loses it wastes the
# whole run.
mutter --headless --virtual-monitor "$VIRTUAL_MONITOR" \
  >"$SHELL_LOG" 2>&1 &
readonly SHELL_PID=$!
echo "mutter pid ${SHELL_PID}"

shell_alive() { kill -0 "$SHELL_PID" 2>/dev/null; }
wayland_socket() { compgen -G "${XDG_RUNTIME_DIR}/wayland-*" >/dev/null; }
# A real client connecting and reading the registry. `wayland-info` exits
# non-zero if it cannot connect, which is the whole point: the socket existing
# and the compositor serving are different claims.
client_connects() {
  WAYLAND_DISPLAY="$(basename "$(compgen -G "${XDG_RUNTIME_DIR}/wayland-[0-9]" | head -1)")" \
    wayland-info >/tmp/wayland-info.txt 2>&1
}

# The process dying is worth catching early and distinctly: "gnome-shell exited"
# and "gnome-shell is up but not on the bus" are different findings with
# different fixes, and a single timeout would conflate them.
if ! shell_alive; then
  fail "mutter exited immediately"
fi

log "waiting for the compositor to listen"
wait_for "a wayland socket in ${XDG_RUNTIME_DIR}" "$STARTUP_TIMEOUT_S" wayland_socket

log "waiting for a real client to connect and read the registry"
wait_for "wayland-info to connect" "$STARTUP_TIMEOUT_S" client_connects

log "what the compositor advertises"
grep -c 'interface:' /tmp/wayland-info.txt | sed 's/^/globals: /'

missing=""
for global in $REQUIRED_GLOBALS; do
  if grep -q "interface: '${global}'" /tmp/wayland-info.txt; then
    printf 'have: %s\n' "$global"
  else
    printf 'MISSING: %s\n' "$global"
    missing="${missing} ${global}"
  fi
done

if [ -n "$missing" ]; then
  fail "the compositor came up but does not advertise:${missing} — a session without these cannot test what this tier is for"
fi

shell_alive || fail "mutter died while being queried"

log "the compositor is up"
echo "A real Wayland compositor runs headless in a container, serves a client,"
echo "and advertises the globals this tier needs. §8.8's Tier 2 is buildable."
echo
echo "NOT proven, and recorded on #119: GNOME Shell itself needs systemd-logind"
echo "and does not run here, so anything Shell-resident still needs a VM."

# With no arguments this is the spike: it proved the premise and stops.
# With arguments it is the tier's harness: bring up a compositor, run the
# thing under test against it, and let that decide the exit code.
#
# One script rather than two so the session a test runs against is the exact
# session the spike verified — a second copy would drift, and the first thing
# to drift would be the readiness checks, which are the part that took two
# runs to get right.
if [ "$#" -eq 0 ]; then
  log "SPIKE PASSED"
  kill "$SHELL_PID" 2>/dev/null || true
  wait "$SHELL_PID" 2>/dev/null || true
  exit 0
fi

WAYLAND_DISPLAY="$(basename "$(compgen -G "${XDG_RUNTIME_DIR}/wayland-[0-9]" | head -1)")"
export WAYLAND_DISPLAY
log "running against the compositor: $*"
echo "WAYLAND_DISPLAY=${WAYLAND_DISPLAY}"

status=0
"$@" || status=$?

# Reported before exiting, because "the command failed" and "the compositor
# died under it" are different findings and the second one is invisible if
# nobody looks.
if ! shell_alive; then
  printf '\nNOTE: the compositor did not survive the command.\n' >&2
  cat "$SHELL_LOG" >&2 || true
fi

kill "$SHELL_PID" 2>/dev/null || true
wait "$SHELL_PID" 2>/dev/null || true
exit "$status"
