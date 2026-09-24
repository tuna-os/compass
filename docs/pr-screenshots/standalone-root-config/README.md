# Standalone root configuration

Captured from the `standalone_search_respects_startup_root_settings` test using
the real Iced/wgpu headless renderer, 800 × 320 logical pixels. Source: the
`codex/standalone-root-config` change based on e92be6feebd218155cb557208d078ed5b87f2b6c.
The fixture assigns Terminal the alias `shellwork`; disabling its entire provider
then hides it despite the entrypoint being explicitly enabled. Both appearances
are captured. These are not screenshots of a GNOME or Flatpak session.

Generate with `COMPASS_UI_SCREENSHOT_DIR=<existing-directory> cargo test -p
compass-ui standalone_search_respects_startup_root_settings`. The renderer adds
`-wgpu` to the generated filenames.

The accompanying browser audit captured all 26 existing states and inspected
light/typing and dark/no-results. Spacing, selection contrast and empty-state
containment remain intact. Those generic fixtures do not exercise standalone
configuration; only the native test above does. No geometry or theme changes
were made. The native input still has a nested border absent in the browser
surrogate, a pre-existing fidelity gap, not a resolved design issue.
