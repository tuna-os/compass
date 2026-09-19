#!/usr/bin/env bash
#
# Prove the Tier 2 smoke can fail. (#119)
#
# WHY THIS EXISTS
#
# A tier that has only ever been green is indistinguishable from one that tests
# nothing. This repository has found that exact defect repeatedly and at cost:
#
#   * `launcher-rss` gated on a figure it read as `0 kB` when /proc was
#     unreadable, and passed;
#   * the `parity` binary was wired into no job at all and "passed" for months;
#   * `cargo bench --bench slas` was named in the pre-flight command against a
#     target that did not exist;
#   * the VM tier's path filter named `spike.rs`, which is a directory, so it
#     matched nothing and the spike could not trigger its own tier.
#
# Every one of those looked green. So a new tier does not get to be trusted on
# the strength of passing — it has to be shown going red for the right reason.
#
# HOW
#
# Run the smoke twice against the same engine, sandbox and compositor:
#
#   with the fixture installed     -> MUST pass
#   with the fixture removed       -> MUST fail
#
# The second run is the control. If the smoke passes without the fixture, its
# assertion is not reading what it claims to read, and the whole tier is
# decorative — so this script fails loudly in that case rather than reporting
# a green smoke.
#
# The two runs differ ONLY in whether the file exists. Nothing else is touched,
# so a difference in outcome is attributable to the fixture and to nothing
# else.

set -euo pipefail

readonly FIXTURE_ID="tier2smoke"
readonly FIXTURE_NAME="Tier2 Smoke Application"
readonly APPS_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/applications"
readonly FIXTURE="${APPS_DIR}/${FIXTURE_ID}.desktop"

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
readonly SMOKE="${here}/smoke.sh"

banner() { printf '\n########## %s ##########\n' "$*"; }

install_fixture() {
  mkdir -p "$APPS_DIR"
  cat > "$FIXTURE" <<DESKTOP
[Desktop Entry]
Type=Application
Name=${FIXTURE_NAME}
Comment=Installed by prove-smoke.sh so the smoke has something to find
Exec=/usr/bin/true
Icon=application-x-executable
Categories=Utility;
DESKTOP
}

banner "RUN 1 of 2: with the fixture, the smoke must PASS"
install_fixture
if "$SMOKE"; then
  echo "as expected: the smoke passes when the fixture is present"
else
  echo "PROOF FAILED: the smoke does not pass even with the fixture installed." >&2
  echo "That is a broken smoke, not a proven one — fix it before trusting this tier." >&2
  exit 1
fi

banner "RUN 2 of 2: without the fixture, the smoke must FAIL"
rm -f "$FIXTURE"
if "$SMOKE"; then
  cat >&2 <<'WHY'
PROOF FAILED: the smoke PASSED with its fixture deleted.

Its assertion is therefore not reading what it claims to. Whatever it is
matching would match with no application indexed at all, which means this tier
would stay green through a sandbox that can read nothing — exactly the failure
it was built to catch.

Do not trust a green run from this tier until this script passes.
WHY
  exit 1
fi
echo "as expected: the smoke fails when the fixture is removed"

banner "PROVEN"
echo "The smoke passes with the fixture and fails without it, so its result"
echo "depends on the thing it claims to measure."
install_fixture
