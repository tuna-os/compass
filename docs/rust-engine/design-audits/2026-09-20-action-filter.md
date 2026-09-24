# Action-filter browser audit — 2026-09-20

Base: `c1aaa52f1e9760a5fe4fec70931a6f7a92d0afcf`, with the changes in this
audit's PR. Generated fresh Rust states and 22 Chromium screenshots using
`make design-states` and `node tools/design/shoot.mjs`.
Local captures: `/tmp/compass-action-audit.LDd1t5`.

The preview now draws the focused action-filter input for empty, matching and
nonmatching filters. It is deliberately read-only: filtering remains the Rust
model's answer, not a second JavaScript search implementation. The main query
no longer draws a second caret while the panel is open.

Visual inspection found something the initial DOM assertions missed: adding
the input made the full panel taller than the card, whose overflow clipped the
input. Focus and the correct value were not evidence that the field was
visible. The preview now bounds the panel, scrolls excess actions and keeps the
filter sticky. Screenshot generation additionally rejects a filter outside
the panel/card bounds. This is a surrogate fix, not proof of native scrolling.

Inspected the final light/dark full panels, dark matching filter and both
light/dark no-action states. The field is visible, the nonmatching state says
“No actions”, and the root query has no misleading focus caret. Full panels
show the start of the overflowing management section; remaining rows can be
scrolled. Fixture management actions are still not evidence of working native
actions. All 22 captures and their DOM assertions pass; loading/error states,
long labels, narrow viewports, native scroll containment and keyboard-selected
row visibility remain separate work.
