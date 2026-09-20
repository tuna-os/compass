# Native search-field border audit

Before: main `3ebee334c5a4598756667bb4fcef64d4790286d0`.
After: the `codex/search-field-border` change based on that commit.

The real Iced/wgpu headless renderer captured the application list at 800 × 320
logical pixels, using `long_root_results_scroll_without_moving_the_query_field`
with `COMPASS_UI_SCREENSHOT_DIR` set to a fresh directory. Both appearances show
the duplicated inner border before the fix and a single enclosing field after.
The input's opaque rectangular background is removed along with its border;
Iced's text, placeholder and selection colours remain unchanged for every status.

A fresh browser audit captured all 26 states. Light and dark long-results were
inspected against the native captures: the browser already had a single field
border, so no surrogate styling changed. The root query glyph and section heading
remain surrogate/native differences. The scrollbar and number of visible rows
differ at these different viewport sizes; this is not a pixel-equivalence claim.

These images are headless Iced/wgpu evidence, not GNOME/Flatpak screenshots.
Existing keyboard, IME, focus and scrolling tests still pass. The new style test
covers active, hovered, focused (hovered/unhovered) and disabled states in light
and dark themes. Target-session CI remains required before merge.
