#!/usr/bin/env bash
# Exercise the installed desktop entry's session command against real Mutter.
# This is not a GNOME Shell app-grid or physical-keyboard test.
set -euo pipefail
: >/tmp/tier2-session.log

readonly APP_ID=com.vicinae.Vicinae
readonly socket="${XDG_RUNTIME_DIR:?}/tier2-session.sock"
readonly timeout_s="${READY_TIMEOUT_S:-60}"
instance_file=$(mktemp /tmp/tier2-session-instance.XXXXXX)
readonly instance_file
session_pid=

fail() {
  printf 'SESSION FAILED: %s\n' "$*" >&2
  exit 1
}
run() { timeout "$timeout_s" flatpak --user run "$APP_ID" --socket "$socket" "$@"; }
cleanup() {
  if [ -s "$instance_file" ]; then
    instance=$(<"$instance_file")
    if [[ "$instance" =~ ^[0-9]+$ ]]; then
      flatpak kill "$instance" 2>/dev/null || true
    fi
  fi
  if [ -n "$session_pid" ]; then
    wait "$session_pid" 2>/dev/null || true
  fi
}
trap cleanup EXIT

[ -n "${WAYLAND_DISPLAY:-}" ] || fail 'a compositor is required'
if run ping >/dev/null 2>&1; then
  fail 'a previous engine still owns the test socket'
fi

location=$(flatpak --user info --show-location "$APP_ID")
desktop="$location/export/share/applications/$APP_ID.desktop"
grep -q '^Exec=.*--command=vicinae .*com.vicinae.Vicinae start$' "$desktop" ||
  fail 'exported entry does not start a session'
if grep -q '^NoDisplay=true' "$desktop"; then
  fail 'exported application is hidden'
fi

for shutdown_mode in engine instance; do
  startup=(start)
  if [ "$shutdown_mode" = instance ]; then
    startup+=(--hidden)
  fi
  # RUST_LOG only widens what is logged: compass_ui::startup carries the
  # summon figure reported at the end. It changes no behaviour under test.
  flatpak --user run --instance-id-fd=3 \
    --env=RUST_LOG=warn,compass_ui::startup=info \
    "$APP_ID" --socket "$socket" "${startup[@]}" \
    3>"$instance_file" >>/tmp/tier2-session.log 2>&1 &
  session_pid=$!
  deadline=$((SECONDS + timeout_s))
  ready=no
  while [ "$SECONDS" -lt "$deadline" ]; do
    kill -0 "$session_pid" 2>/dev/null || {
      cat /tmp/tier2-session.log >&2
      fail 'session exited during startup'
    }
    if run show >/dev/null 2>&1; then
      ready=yes
      break
    fi
    sleep 1
  done
  [ "$ready" = yes ] || fail 'resident window never acknowledged Show'

  # A duplicate renderer would remain running and trip timeout, not pass here.
  run start || fail 'repeat activation did not return to the existing session'
  run hide || fail 'resident window did not acknowledge Hide'
  run show || fail 'resident window did not reopen'
  run hide || fail 'resident window did not hide again'

  instance=$(<"$instance_file")
  [[ "$instance" =~ ^[0-9]+$ ]] || fail 'Flatpak did not report its instance ID'
  if [ "$shutdown_mode" = engine ]; then
    run shutdown || fail 'engine-only shutdown failed'
    deadline=$((SECONDS + timeout_s))
    while kill -0 "$session_pid" 2>/dev/null && [ "$SECONDS" -lt "$deadline" ]; do
      sleep 1
    done
    if kill -0 "$session_pid" 2>/dev/null; then
      fail 'window process survived engine-only shutdown'
    fi
  else
    flatpak kill "$instance"
  fi
  wait "$session_pid" 2>/dev/null || true
  session_pid=
  if run ping >/dev/null 2>&1; then
    fail "engine survived $shutdown_mode shutdown"
  fi
done
printf 'SESSION PASSED: startup, repeat activation, hide/show, engine shutdown, restart and instance shutdown\n'

# SUMMON, RECORDED (§8.5, ADR-0010): each hidden-to-shown above logs how long
# the window took from the engine's Show to its first redraw request, on real
# Mutter. A floor on summon latency -- the paint after the request is not in
# it -- and reported, not gated, until runs show what the number is.
summons=$(sed 's/\x1b\[[0-9;]*m//g' /tmp/tier2-session.log |
  sed -n 's/.*summon_draw_ms=\([0-9]\+\).*/\1/p' | paste -sd' ' -)
if [ -n "$summons" ]; then
  printf 'summon to first frame requested (ms, each summon in order): %s\n' "$summons"
else
  printf 'summon figure NOT LOGGED: either the window process does not inherit\n'
  printf 'RUST_LOG inside the sandbox, or its stderr is not in this log.\n'
fi
