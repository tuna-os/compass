# System color-scheme default

The color-mode change has no geometry delta. These existing native Iced/wgpu
captures show the light and dark palettes that the System mode selects:

- [light launcher render](../live-launcher-alignment/light-native-wgpu.png)
- [dark launcher render](../live-launcher-alignment/dark-native-wgpu.png)

They are real headless wgpu renders, not browser screenshots. The browser
surrogate audit is recorded in
[`docs/rust-engine/design-audits/2026-09-20-system-theme.md`](../../rust-engine/design-audits/2026-09-20-system-theme.md).
The VM/Flatpak tier remains authoritative for portal delivery and a live
desktop appearance change.
