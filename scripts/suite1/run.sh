#!/usr/bin/env bash
# Runs Suite 1's harness against an installed corpus, on the host.
#
#   scripts/suite1/run.sh <vicinae> <extensions-dir> <plan.json> <report.json> [timeout]
#
# Everything the engine touches is a fresh temporary tree: data, config,
# cache, state, runtime directory and HOME. The extensions are linked in as
# $XDG_DATA_HOME/vicinae/extensions. A private session bus with an unlocked
# gnome-keyring stands in for the login keyring, because preferences are
# kept encrypted under a key from it and the engine refuses to run a command
# whose required preferences it has nowhere safe to keep. Nothing here can
# reach the invoking user's own bus, keyring or files.
#
# Needs dbus-run-session and gnome-keyring-daemon, Node on PATH (or
# COMPASS_NODE), the runtime bundle (COMPASS_EXTENSION_RUNTIME, or beside the
# binary) and compass-sandbox-exec beside the binary.
set -euo pipefail

vicinae="$(realpath "$1")"
extensions="$(realpath "$2")"
plan="$(realpath "$3")"
report="$(realpath -m "$4")"
timeout="${5:-30}"

root="$(mktemp -d)"
trap 'rm -rf "$root"' EXIT
export XDG_DATA_HOME="$root/data"
export XDG_CONFIG_HOME="$root/config"
export XDG_CACHE_HOME="$root/cache"
export XDG_STATE_HOME="$root/state"
export XDG_RUNTIME_DIR="$root/run"
export XDG_DATA_DIRS="$root/share"
export HOME="$root/home"
mkdir -p "$XDG_DATA_HOME/vicinae" "$XDG_CONFIG_HOME" "$XDG_CACHE_HOME" "$XDG_STATE_HOME" \
  "$XDG_RUNTIME_DIR" "$XDG_DATA_DIRS" "$HOME"
chmod 700 "$XDG_RUNTIME_DIR"
ln -s "$extensions" "$XDG_DATA_HOME/vicinae/extensions"

# A browser that only writes down what it was asked to open. A command that
# signs in with OAuth then waits on it, as it would on a person, and the
# harness reports that rather than a crash for want of a browser.
mkdir -p "$XDG_DATA_DIRS/applications"
cat > "$root/browser.sh" <<BROWSER
#!/bin/sh
printf '%s\n' "\$1" >> "$root/opened.txt"
BROWSER
chmod +x "$root/browser.sh"
cat > "$XDG_DATA_DIRS/applications/suite1-browser.desktop" <<ENTRY
[Desktop Entry]
Type=Application
Name=Suite 1 Browser
Exec=$root/browser.sh %u
MimeType=x-scheme-handler/http;x-scheme-handler/https;
ENTRY

# The keyring asks for its password on stdin, and creates and unlocks a
# login keyring in the fresh HOME with it. Not an empty one: a keyring with
# no password asks a prompter for one, and there is no display to show it.
export VICINAE="$vicinae" PLAN="$plan" REPORT="$report" TIMEOUT="$timeout"
dbus-run-session -- sh -c '
  printf compass | gnome-keyring-daemon --unlock --components=secrets >/dev/null
  exec "$VICINAE" conformance --plan "$PLAN" --timeout "$TIMEOUT" --json > "$REPORT"
' || status=$?
echo "report at $report"
exit "${status:-0}"
