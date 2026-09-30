#!/usr/bin/env bash
#
# Tier 2, next question: can the Flatpak we ship install and run in a
# container, against the headless compositor the spike proved? (#119)
#
# Run BY `headless-gnome-spike.sh`, which brings the compositor up first and
# exports WAYLAND_DISPLAY. That is why this script asserts the display is set
# rather than setting one: if it ever runs standalone, it should say so instead
# of quietly testing nothing.
#
# WHY `--version` AND NOT THE LAUNCHER
#
# Smallest question first, the same way the compositor spike started with "does
# a socket appear" rather than "does the launcher paint". Flatpak needs
# bubblewrap, bubblewrap needs user namespaces, and a container may not grant
# them. If `flatpak run` cannot execute at all, discovering that through a
# launcher that fails to draw would be a slow and confusing way to learn it.
#
# The launcher comes next, once this says the sandbox works here.

set -euo pipefail

readonly BUNDLE_DIR="${BUNDLE_DIR:-/tmp/bundle}"

log() { printf '\n=== %s ===\n' "$*"; }
fail() { printf 'PROBE FAILED: %s\n' "$*" >&2; exit 1; }

[ -n "${WAYLAND_DISPLAY:-}" ] \
  || fail "WAYLAND_DISPLAY is not set — this is meant to run under headless-gnome-spike.sh, which brings a compositor up first"

log "what we are installing"
bundle="$(find "$BUNDLE_DIR" -name '*.flatpak' -type f | head -1)"
[ -n "$bundle" ] || fail "no .flatpak bundle under ${BUNDLE_DIR} — the artifact did not arrive"
ls -l "$bundle"

log "does bubblewrap work here at all?"
# Asked directly, before Flatpak can obscure it. bwrap failing to create a user
# namespace is THE likely reason this whole approach does not work in a
# container, and its own error message is far clearer than Flatpak's.
bwrap --ro-bind / / --dev /dev --unshare-user --unshare-pid true \
  || fail "bubblewrap cannot create a sandbox in this container. Flatpak cannot work here without more privilege — that is the finding, not a bug in the bundle"
echo "ok: bubblewrap made a sandbox"

log "installing the bundle"
flatpak --user remote-add --if-not-exists flathub https://dl.flathub.org/repo/flathub.flatpakrepo
flatpak --user install -y --bundle "$bundle" \
  || fail "the bundle would not install — read the error above; a missing runtime is a different problem from a broken sandbox"

log "what is installed"
flatpak --user list --columns=application,version,branch,installation

log "running it"
# --version rather than the launcher: see the header. It still exercises the
# whole sandbox path — bwrap, the runtime, the app's entrypoint — which is what
# is actually in question.
flatpak --user run --command=compass org.tunaos.compass --version \
  || fail "the Flatpak installed but would not run"

log "PROBE PASSED"
echo "The Flatpak installs and runs inside a container, with a Wayland display"
echo "available. Driving the launcher itself is the next step."
