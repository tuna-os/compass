# Local icon overlays: Antigravity lookup regression

The running desktop entry uses `Icon=antigravity-ide`. Its local hicolor
directory contains the PNG but no `index.theme`. The previous resolver required
metadata in each base directory, skipping this valid overlay.

The [icon theme specification, directory layout](https://specifications.freedesktop.org/icon-theme/latest/#directory_layout)
defines one theme across multiple base directories, using the first index found.
The resolver now loads that index once per theme and searches every base using
it. Inheritance comes from the same index, in declared order; hicolor is searched
before the existing arbitrary-theme fallback. This does not claim complete
specification compliance for size/scale selection or unthemed icons.

## Local sandbox verification

Compiled an optimized probe against the actual compass-xdg library, then ran it
inside the installed `com.vicinae.Vicinae` Flatpak with only the probe's temporary
directory additionally exposed read-only. No desktop entries, icon files, theme
metadata, or persistent sandbox permissions were changed.

Before: `antigravity-ide: unresolved`.

After: resolves to the sandbox's mapped
`data/icons/hicolor/512x512/apps/antigravity-ide.png`; **249650 bytes readable**.
Also checked the actual desktop icon names for Bazaar, Chromium, Disk Usage
Analyzer and Disks: all resolved and were readable. An initial probe using
`chromium` did not resolve; its real desktop icon name is `org.chromium.Chromium`,
which does resolve.

This is filesystem/lookup evidence, not a screenshot or confirmation that the
installed UI is already running the patched resolver. Native display verification
remains necessary after installing the new build.

## Automated coverage

- Local overlay without metadata resolves using the system index and overrides
  the same icon in the system root.
- The first index controls directory metadata and inheritance across roots.
- Inherited themes and hicolor fallback find metadata-free local overlays;
  inheritance cycles terminate.
- Existing XDG crate tests and warning-denying Clippy pass.

No general speed or memory claim is derived from these one-shot probes.
