#!/usr/bin/env bash
# The launcher on a real wlroots compositor: headless Sway, in-process.
#
# The wlroots counterpart of scripts/tier2 (real Mutter). It starts a headless
# Sway in a private runtime directory, an engine, and a resident launcher, and
# gates on three things only a compositor can answer:
#
#   1. the launcher appears on screen (a screenshot of the output changes),
#   2. it is a LAYER SURFACE, not a window: Sway's tree lists every
#      xdg_toplevel by app_id and must not list org.tunaos.compass,
#   3. `compass toggle` hides it and shows it again, on screen,
#   4. (with wtype) typed text reaches it: a layer surface takes the keyboard.
#
# Every gate is proven able to fail by its control: COMPASS_LAYER_SHELL=0
# forces the xdg_toplevel surface, and then gate 2 must fail. A gate that has
# only ever passed is indistinguishable from one that checks nothing.
#
# Usage: scripts/wlroots/launcher-on-sway.sh [path/to/compass]
# Needs: sway, grim, swaymsg, python3; wtype for gate 4. Sway renders with
# pixman (no GPU).
set -euo pipefail

readonly COMPASS=${1:-target/debug/compass}
readonly APP_ID=org.tunaos.compass
readonly WAIT_S=${WAIT_S:-60}

for tool in sway swaymsg grim python3; do
  command -v "$tool" >/dev/null || { echo "missing: $tool" >&2; exit 2; }
done
[ -x "$COMPASS" ] || { echo "no compass binary at $COMPASS (cargo build -p compass)" >&2; exit 2; }

# COMPASS_WLROOTS_WORK keeps everything (logs, screenshots) in a known place,
# for CI to upload; otherwise a temporary directory removed on exit.
work=${COMPASS_WLROOTS_WORK:-$(mktemp -d /tmp/compass-wlroots.XXXXXX)}
readonly work
mkdir -p "$work"
[ -n "${COMPASS_WLROOTS_WORK:-}" ] && KEEP_LOGS=1
pids=()
cleanup() {
  for pid in "${pids[@]}"; do kill "$pid" 2>/dev/null || true; done
  wait 2>/dev/null || true
  if [ "${KEEP_LOGS:-0}" = 1 ]; then echo "logs kept in $work"; else rm -rf "$work"; fi
}
trap cleanup EXIT

mkdir -p -m 700 "$work/run"
mkdir -p "$work/home" "$work/config" "$work/data" "$work/cache"
export XDG_RUNTIME_DIR="$work/run" HOME="$work/home" XDG_CONFIG_HOME="$work/config" \
  XDG_DATA_HOME="$work/data" XDG_CACHE_HOME="$work/cache" XDG_CURRENT_DESKTOP=sway
unset WAYLAND_DISPLAY DISPLAY SWAYSOCK

printf 'output HEADLESS-1 resolution 1280x800 bg #202020 solid_color\nxwayland disable\n' \
  >"$work/sway.cfg"
WLR_BACKENDS=headless WLR_LIBINPUT_NO_DEVICES=1 WLR_RENDERER=pixman \
  sway -c "$work/sway.cfg" >"$work/sway.log" 2>&1 &
pids+=($!)

for _ in $(seq 1 200); do
  WAYLAND_DISPLAY=$(cd "$XDG_RUNTIME_DIR" && ls wayland-* 2>/dev/null | grep -v lock | head -1) || true
  SWAYSOCK=$(ls "$XDG_RUNTIME_DIR"/sway-ipc.*.sock 2>/dev/null | head -1) || true
  [ -n "$WAYLAND_DISPLAY" ] && [ -n "$SWAYSOCK" ] && break
  sleep 0.1
done
[ -n "$WAYLAND_DISPLAY" ] || { cat "$work/sway.log" >&2; echo "sway never listened" >&2; exit 1; }
export WAYLAND_DISPLAY SWAYSOCK

# Share of pixels that differ from the background colour, in the centre third.
covered() {
  grim -t ppm "$work/shot.ppm"
  python3 - "$work/shot.ppm" <<'PY'
import sys
data = open(sys.argv[1], 'rb').read()
parts, pos = [], 0
while len(parts) < 4:
    while data[pos:pos+1].isspace(): pos += 1
    if data[pos:pos+1] == b'#':
        pos = data.index(b'\n', pos); continue
    end = pos
    while not data[end:end+1].isspace(): end += 1
    parts.append(data[pos:end]); pos = end
pos += 1
w, h = int(parts[1]), int(parts[2])
px = data[pos:]
bg = (0x20, 0x20, 0x20)
hit = total = 0
for y in range(h // 3, 2 * h // 3, 4):
    for x in range(w // 3, 2 * w // 3, 4):
        i = 3 * (y * w + x)
        total += 1
        if tuple(px[i:i+3]) != bg: hit += 1
print(f"{hit / total:.3f}")
PY
}

in_tree() { swaymsg -t get_tree | grep -c "\"app_id\": \"$APP_ID\"" || true; }

wait_for() { # wait_for <description> <command...>
  local what=$1; shift
  for _ in $(seq 1 $((WAIT_S * 10))); do "$@" && return 0; sleep 0.1; done
  echo "TIMED OUT: $what" >&2
  return 1
}
digest() { grim -t ppm - | sha1sum | cut -d' ' -f1; }
changed_from() { [ "$(digest)" != "$1" ]; }
settled() { # the same frame across one second
  local first; first=$(digest); sleep 1; [ "$(digest)" = "$first" ]
}
quiet() { "$@" >/dev/null 2>&1; }
shown() { [ "$(covered)" != "0.000" ]; }
hidden() { [ "$(covered)" = "0.000" ]; }

run_session() { # run_session <label> <expect layer: yes|no>
  local label=$1 expect_layer=$2 socket="$work/$1.sock"
  RUST_LOG=${RUST_LOG:-info} "$COMPASS" --socket "$socket" serve --no-hotkey \
    >"$work/$label-engine.log" 2>&1 &
  local engine=$!; pids+=("$engine")
  wait_for "engine" quiet "$COMPASS" --socket "$socket" ping
  COMPASS_NO_ONBOARDING=1 RUST_LOG=${RUST_LOG:-info} "$COMPASS" --socket "$socket" ui >"$work/$label-ui.log" 2>&1 &
  local ui=$!; pids+=("$ui")

  hidden || { echo "[$label] the output is not empty before the launcher" >&2; return 1; }
  wait_for "[$label] launcher on screen" shown || { tail -20 "$work/$label-ui.log" >&2; return 1; }
  echo "[$label] gate 1: launcher on screen (centre coverage $(covered))"
  grim "$work/$label-shown.png"

  # Gate 4 (optional, needs wtype): typing reaches the search field. For the
  # layer surface this is the point of `keyboard_interactivity = exclusive`:
  # a launcher that is on screen but deaf is the failure users would see.
  #
  # The baseline is taken once the screen has stopped changing: the root list
  # fills in asynchronously, and a first version of this gate passed on that
  # alone while the keys went nowhere.
  if command -v wtype >/dev/null; then
    wait_for "[$label] a settled frame" settled
    local before; before=$(digest)
    wtype 'xyzzy-no-such-app'
    wait_for "[$label] typed text on screen" changed_from "$before" ||
      { echo "[$label] gate 4 FAILED: typing did not reach the launcher" >&2; return 1; }
    grim "$work/$label-typed.png"
    echo "[$label] gate 4: typed text reached the search field"
  else
    echo "[$label] gate 4 SKIPPED: no wtype"
  fi

  local count; count=$(in_tree)
  if [ "$expect_layer" = yes ]; then
    [ "$count" = 0 ] || { echo "[$label] gate 2 FAILED: launcher is an xdg_toplevel" >&2; return 1; }
    grep -q "wlr-layer-shell" "$work/$label-ui.log" ||
      { echo "[$label] gate 2 FAILED: the UI did not choose the layer shell" >&2; return 1; }
    echo "[$label] gate 2: a layer surface, not in Sway's window tree"
  else
    [ "$count" != 0 ] || { echo "[$label] control FAILED: toplevel not in tree" >&2; return 1; }
    echo "[$label] control: forced xdg_toplevel is in Sway's window tree, as it must be"
  fi

  "$COMPASS" --socket "$socket" toggle >/dev/null
  wait_for "[$label] hidden after toggle" hidden
  "$COMPASS" --socket "$socket" toggle >/dev/null
  wait_for "[$label] shown after second toggle" shown
  echo "[$label] gate 3: toggle hid and re-showed it"

  "$COMPASS" --socket "$socket" shutdown >/dev/null 2>&1 || true
  kill "$ui" "$engine" 2>/dev/null || true
  wait "$ui" "$engine" 2>/dev/null || true
  wait_for "[$label] output empty again" hidden
}

run_session layer yes
COMPASS_LAYER_SHELL=0 run_session toplevel no
echo "PASS: the launcher is a layer surface on Sway, and the control can tell the difference"
