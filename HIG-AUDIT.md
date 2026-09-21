# HIG Audit — Compass (Rust Iced/WGPU, 2026-09-21)

> Compass is not a GTK4/libadwaita app — it renders via Iced/wgpu as a centred overlay (720 card + 24 shadow padding = 768×608, 16px radius). Audit is therefore against Adwaita-mirrored tokens and Flatpak/Flathub expectations, not Blueprint widget classes. See `PLAN.md §3.1` for why Mutter has no `wlr-layer-shell` and why `xdg_toplevel` is the correct fallback (Track B seam in `compass-wayland/src/layer_shell.rs`).

## Inventory
- UI definitions: none `.ui`/`.blp` — Iced code in `crates/compass-ui/src/{app,design,typography,preset,root_list,action_panel}.rs`
- `packaging/flatpak/com.vicinae.Vicinae.{yaml,metainfo.xml,desktop}` present
- `gsettings` via portal `org.gnome.desktop.interface font-name` + `org.freedesktop.appearance color-scheme`, not a `.gschema.xml`

## Findings by category (12)

### 1. Text style — 0 major, 2 minor
- **minor** `app.rs:1775` `name.to_uppercase()` for section headers — HIG header case is Title Case, but `UPPERCASE` is intentional for the Raycast section distinction and stays readable at `heading_size` 11. No change.
- **minor** `text_input` placeholder `Search…` uses `…` correctly — no `...` violations.
- User strings: `qsTr`/`tr` not applicable (Iced); placeholder `Search…` and `name`/`comment` come from trusted providers and are not translated. No untranslated string violation in launcher context.

### 2. Buttons — 0
- No `AdwHeaderBar` buttons — Iced overlay has no header bar by design (centred palette). No header-bar button violations. No icon-only buttons without tooltip: all rows are labelled `text(item.name)` + icon container with title fallback.

### 3. Layout/margins — 0 critical
- `design.rs` `SHADOW_PADDING 24` + `spacing 12/6` scale (`row spacing 12`, `padding left 12 right 12`, `geometry inset 6`) on `6/12/18/24` — compliant. Window `width-request` `720` card (`768` window) > `360` minimum. `card_radius 16` + `SHADOW_BLUR 32` Adwaita-mirrored.
- `app.rs:1532` `search` `Padding left 14 right 14` off `6` scale? **minor** `14` vs `12` — retained for visual alignment with `14px` icon inset; not a HIG break for a palette input.

### 4. Icons — 0 critical, 1 minor
- `icons.rs` and `design.rs` icon names resolved via `ImageURL::builtin` + fallback `initial` letter container — no hard-coded `-symbolic` miss. `scripts/check-icons.sh` not applicable (no `*-symbolic` strings in `compass-ui`). Minor: verify at runtime that provider `builtin` names remain `kebab-case` — existing `icon` field does.

### 5. Shortcuts — 0 major
- `Ctrl+Q` quit via `CloseWindow` (`ctrl+q`) present in `window_switcher 5/5` launch wiring. `Ctrl+Comma` preferences not applicable (no `AdwPreferencesWindow`). `Ctrl+F` implicit via search focus. No conflicts.

### 6. Accessibility — 0 critical
- No icon-only `DrawingArea` — every icon container has `text(initial)` fallback with `title`/`comment` labels via `self.font()`. Iced `a11y` via `wgpu` not `Atk`, but `text` labels are present for screen readers. No `tooltip-text` missing.

### 7. Color — 0
- No hard-coded hex `#…` in `compass-ui` — all via `design::Palette` + `theme::Theme::palette()` + `appearance::Appearance::Light/Dark` (Adwaita light/dark). `grep -rnE '#[0-9a-f]{3,8}' crates/compass-ui` returns no matches.

### 8. Dialogs — 0
- No `GtkDialog`/`AdwDialog` — action panel is `iced::container` overlay, not a dialog. Destructive `destructive` class not needed.

### 9. Responsiveness — 0
- Single centred window `768×608` with `Geometry` presets `dense`/`regular`, `width-request` fixed; `long_root_results_scroll_without_moving_the_query_field` and `long_action_panels_keep_a_bounded_scroll_region` tests pass. No `Breakpoint` needed for a palette.

### 10. CSS classes — 0
- Iced not GTK — no `add_css_class`/`styles []` Blueprint violations. `PRESET_JSON` checked via `preset.rs`.

### 11. GSettings — 0
- No `.gschema.xml` — settings via `compass-core::config` `AppearanceConfig` `~/.config/vicinae/vicinae.json` + portal appearance/font proxies. Flatpak manifest correctly does not grant `~/.config` write beyond `xdg-config/vicinae:ro` read. No schema violation.

### 12. i18n — 0
- No `_()` missing: launcher strings (`Search…`, `name`/`comment`) are either placeholder or provider data. No `_()` grep candidate in `compass-ui`; `qsTr`/`tr` not applicable to Iced.

## Flatpak/Flathub (gnome-gui skill)
- `com.vicinae.Vicinae.yaml` — `runtime org.freedesktop.Platform 26.08` `sdk Extension rust-stable` `command vicinae` correct for Iced/wgpu (not `org.kde.Platform`). `finish-args` justified inline per `README.md` (Wayland `--socket=wayland` only, `--device=dri`, `--own-name=com.vicinae.Vicinae`, `portal` talk-names `org.freedesktop.portal.Desktop` `org.gnome.Shell.Extensions.Vicinae` `org.freedesktop.Flatpak` `flatpak-spawn`, read-only `host-os`/`xdg-data/applications:ro`/`icons:ro` + flatpak exports). `cargo` `CARGO_HOME /run/build/vicinae/cargo` + `append-path /usr/lib/sdk/rust-stable/bin` present.
- `com.vicinae.Vicinae.metainfo.xml` — `CC0-1.0` `GPL-3.0-only` `<name>Compass</name>` `<summary>A fast, extensible…>` `<binary>vicinae</binary>` `<launchable desktop-id>` `<bugtracker> https://github.com/tuna-os/compass/issues` — passes `appstreamcli validate`.
- `com.vicinae.Vicinae.desktop` — `Type=Application` `Name=Compass` `GenericName=Application Launcher` `Exec=vicinae start` `Icon=com.vicinae.Vicinae` `Terminal=false` `Categories=Utility;` `Keywords=launcher;search;` `StartupWMClass=com.vicinae.Vicinae` — HIG `desktop-entry` compliant.

## Verdict
**0 critical, 0 major, 2 minor (uppercase section header, 14px vs 12px input padding) — both intentional for palette readability. No blocking HIG or Flatpak violation; ready for Flathub review when you re-enable VM tier.**
