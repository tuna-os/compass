# Live launcher feedback: alignment and icons

Addresses the live local report in #147 and #149, following the search-field
border fix in #146. Native evidence comes from
`mixed_result_rows_center_their_labels_and_render_resolved_icons`, using the real
Iced/wgpu headless renderer at 800 × 320 logical pixels. The temporary fixture
supplies a resolved SVG icon and two entries, one with a description and one
without. These are deterministic fixture icons, not proof that every host app's
icon can be resolved inside Flatpak.

The test checks label-block vertical centering across all four presets, in both
appearances. The captures here show the default GNOME appearance. The icon-off
option and missing-icon fallback remain supported; Rofi still defaults to icons
off. GNOME now defaults to icons on, updating the older #85 decision in response
to the user's live testing feedback. No user configuration is rewritten.

The fresh browser audit captured all 26 states. Inspected light GNOME, dark
Raycast, light Flow and dark Rofi preset frames: layout and selection contrast
remain intact. The browser continues to use initial tiles instead of resolving
the host icon theme; it cannot validate the requested real-icon behavior.

Native keyboard/IME/scrolling tests remain required, as do the GNOME/Flatpak CI
checks. This is not a claim that the wider #110 appearance issue is complete.
