# Native color-scheme default audit

This audit accompanies the `launcher.appearance.color_scheme` change for #152.
An absent setting now means `system`: the launcher reads the XDG Settings
portal, follows later light/dark changes, and falls back to Adwaita light when
the portal is unavailable. Explicit `light` and `dark` values skip the watcher
and remain fixed across desktop changes.

The fast browser surrogate was run with:

```sh
PLAYWRIGHT_ROOT=/var/home/james/.npm/_npx/e41f203b7505f1fb/node_modules/playwright \
CHROMIUM_PATH=/var/home/james/.cache/ms-playwright/chromium-1243/chrome-linux64/chrome \
node tools/design/shoot.mjs /tmp/compass-system-theme-browser
```

All 18 state captures and all eight preset captures completed. The inspected
`light-typing` and `dark-typing` frames keep the same geometry and selection
contrast while changing only the palette. The browser is evidence for layout
and color tokens; it is not evidence of portal delivery.

Native Iced/wgpu light/dark captures remain in
[`live-launcher-alignment`](../../pr-screenshots/live-launcher-alignment/README.md).
The mode-selection tests additionally assert that fixed overrides do not create
an appearance subscription, while the System path retains the live channel.
GNOME portal delivery and a live desktop toggle remain integration-tier work.
