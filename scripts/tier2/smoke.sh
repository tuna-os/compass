#!/usr/bin/env bash
#
# Tier 2 smoke: does the shipped Flatpak index and find an application? (#119)
#
# Run BY `headless-gnome-spike.sh`, which brings a headless compositor up and
# exports WAYLAND_DISPLAY first.
#
# WHAT THIS EXERCISES THAT TIER 1 CANNOT
#
# The sandbox. Tier 1 runs the engine as an ordinary process with the runner's
# own filesystem; this runs the artifact users install, inside bubblewrap, with
# only the permissions `com.vicinae.Vicinae.yaml` grants. A desktop entry the
# engine can read on a developer's machine and not through
# `--filesystem=xdg-data/applications:ro` is a bug Tier 1 cannot see and users
# would hit immediately.
#
# So the fixture goes where a real user's entries live — $XDG_DATA_HOME/
# applications — and is found, or not, through exactly the permission the
# manifest grants.
#
# WHY THE ASSERTION IS THE FIXTURE'S NAME AND NOT "SOME RESULTS CAME BACK"
#
# A query returning *something* proves the socket works. It does not prove the
# sandbox can read the directory the result should come from — an engine that
# indexed nothing and returned an empty list would pass "did it answer".

set -euo pipefail

readonly APP_ID="com.vicinae.Vicinae"
readonly FIXTURE_ID="tier2smoke"
readonly FIXTURE_NAME="Tier2 Smoke Application"
readonly READY_TIMEOUT_S="${READY_TIMEOUT_S:-60}"

log() { printf '\n=== %s ===\n' "$*"; }
fail() { printf 'SMOKE FAILED: %s\n' "$*" >&2; exit 1; }

[ -n "${WAYLAND_DISPLAY:-}" ] \
  || fail "WAYLAND_DISPLAY is not set — run this under headless-gnome-spike.sh"

# Inside the sandbox this path is what `--filesystem=xdg-data/applications:ro`
# exposes. Outside it is an ordinary user directory. That correspondence is the
# whole point of the test.
readonly APPS_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/applications"

run_in_flatpak() {
  flatpak --user run --command=vicinae "$APP_ID" "$@"
}

log "the fixture the sandbox should be able to see"
if [ -f "${APPS_DIR}/${FIXTURE_ID}.desktop" ]; then
  echo "present: ${APPS_DIR}/${FIXTURE_ID}.desktop"
else
  # Not an error here. `prove-smoke.sh` deliberately runs this with the fixture
  # removed and REQUIRES the smoke to fail, which is what demonstrates the
  # assertion below is load-bearing.
  echo "absent: ${APPS_DIR}/${FIXTURE_ID}.desktop (the query below should find nothing)"
fi

log "starting the engine inside the sandbox"
socket="${XDG_RUNTIME_DIR}/tier2-smoke.sock"
rm -f "$socket"
run_in_flatpak --socket "$socket" serve >/tmp/tier2-engine.log 2>&1 &
engine_pid=$!

cleanup() {
  kill "$engine_pid" 2>/dev/null || true
  wait "$engine_pid" 2>/dev/null || true
}
trap cleanup EXIT

# Polled, not slept: an engine still indexing has not finished allocating or
# reading, and a fixed sleep would make this pass or fail on runner speed.
deadline=$((SECONDS + READY_TIMEOUT_S))
ready=no
while [ "$SECONDS" -lt "$deadline" ]; do
  if run_in_flatpak --socket "$socket" ping >/dev/null 2>&1; then
    ready=yes
    break
  fi
  sleep 1
done

if [ "$ready" != yes ]; then
  cat /tmp/tier2-engine.log >&2 || true
  fail "the engine never answered a ping inside the sandbox"
fi
echo "ok: the engine answers on ${socket}"

log "querying for the fixture"
results="$(run_in_flatpak --socket "$socket" query --json "$FIXTURE_ID" 2>/tmp/tier2-query.err)" || {
  cat /tmp/tier2-query.err >&2 || true
  fail "the query command itself failed"
}
echo "$results"

# The name, not the count. See the header.
printf '%s' "$results" | grep -q "$FIXTURE_NAME" \
  || fail "the sandboxed engine did not find '${FIXTURE_NAME}'. Either it cannot read ${APPS_DIR} through the manifest's permissions, or it indexed nothing"

log "SMOKE PASSED"
echo "The shipped Flatpak indexed a desktop entry through its own sandbox"
echo "permissions and returned it for a query."
