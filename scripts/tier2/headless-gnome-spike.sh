#!/usr/bin/env bash
#
# Tier 2 spike: does GNOME start headless in a container? (#119)
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
# WHAT IT ASSERTS, AND WHY EACH ONE
#
# A spike that only checks "the command did not immediately exit" would pass
# against a shell that crashed a second later, which is the class of check this
# repository keeps having to fix. So it asserts three things, each strictly
# stronger than the last:
#
#   1. gnome-shell is installed and reports a version — rules out a package
#      that silently is not there.
#   2. a Wayland socket appears in XDG_RUNTIME_DIR — the compositor got far
#      enough to listen.
#   3. org.gnome.Shell answers on the session bus — the shell is actually up
#      and serving, not merely a process that has not died yet. This is the one
#      that matters; the first two are diagnostics for when it fails.
#
# Nothing here sleeps a plausible number of seconds and hopes. Each wait polls
# for a condition with a deadline, and says which condition it gave up on.

set -euo pipefail

readonly VIRTUAL_MONITOR="${VIRTUAL_MONITOR:-1280x800}"
readonly STARTUP_TIMEOUT_S="${STARTUP_TIMEOUT_S:-60}"

log() { printf '\n=== %s ===\n' "$*"; }

fail() {
  printf 'SPIKE FAILED: %s\n' "$*" >&2
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
gnome-shell --version || fail "gnome-shell is not installed"
echo "fedora: $(rpm -E %fedora 2>/dev/null || echo unknown)"
echo "mutter: $(rpm -q mutter 2>/dev/null || echo 'not installed')"
echo "portal: $(rpm -q xdg-desktop-portal-gnome 2>/dev/null || echo 'not installed')"

log "starting a session bus"
export XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-/run/user/0}"
mkdir -p "$XDG_RUNTIME_DIR"
chmod 700 "$XDG_RUNTIME_DIR"

eval "$(dbus-launch --sh-syntax)"
export DBUS_SESSION_BUS_ADDRESS
echo "bus: ${DBUS_SESSION_BUS_ADDRESS}"

log "starting gnome-shell --headless --virtual-monitor ${VIRTUAL_MONITOR}"
# Logged to a file rather than discarded: when this fails, the shell's own
# output is the only thing that says why, and a spike that loses it wastes the
# whole run.
gnome-shell --headless --virtual-monitor "$VIRTUAL_MONITOR" \
  >/tmp/gnome-shell.log 2>&1 &
readonly SHELL_PID=$!
echo "gnome-shell pid ${SHELL_PID}"

shell_alive() { kill -0 "$SHELL_PID" 2>/dev/null; }
wayland_socket() { compgen -G "${XDG_RUNTIME_DIR}/wayland-*" >/dev/null; }
shell_on_bus() {
  gdbus introspect --session --dest org.gnome.Shell \
    --object-path /org/gnome/Shell >/dev/null 2>&1
}

# The process dying is worth catching early and distinctly: "gnome-shell exited"
# and "gnome-shell is up but not on the bus" are different findings with
# different fixes, and a single timeout would conflate them.
if ! shell_alive; then
  cat /tmp/gnome-shell.log >&2 || true
  fail "gnome-shell exited immediately"
fi

log "waiting for the compositor to listen"
wait_for "a wayland socket in ${XDG_RUNTIME_DIR}" "$STARTUP_TIMEOUT_S" wayland_socket

log "waiting for org.gnome.Shell on the session bus"
if ! wait_for "org.gnome.Shell to answer" "$STARTUP_TIMEOUT_S" shell_on_bus; then
  cat /tmp/gnome-shell.log >&2 || true
  exit 1
fi

log "the session is up"
echo "wayland sockets: $(compgen -G "${XDG_RUNTIME_DIR}/wayland-*" | tr '\n' ' ')"
echo "shell still alive: $(shell_alive && echo yes || echo NO)"

# The shell has to survive being asked something, not merely have appeared. A
# compositor that answers one introspect and then dies would pass everything
# above.
log "asking the shell for its monitor layout"
gdbus call --session --dest org.gnome.Shell \
  --object-path /org/gnome/Shell \
  --method org.freedesktop.DBus.Properties.Get \
  org.gnome.Shell ShellVersion || fail "the shell stopped answering"

shell_alive || fail "gnome-shell died while being queried"

log "SPIKE PASSED"
echo "Headless GNOME starts in a container and serves its bus name."
echo "PLAN.md §8.8's Tier 2 premise holds on a hosted runner; #119 can build on it."

kill "$SHELL_PID" 2>/dev/null || true
wait "$SHELL_PID" 2>/dev/null || true
