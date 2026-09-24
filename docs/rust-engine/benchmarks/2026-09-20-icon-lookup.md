# Local icon lookup investigation

Enabling native application icons in #150 exposed repeated work in the icon
theme parser. A directory section such as `32x32/apps` was compared against only
the filename `apps`. Each key therefore created another directory entry rather
than updating the original, separating size/type/context metadata and multiplying
the lookup work. A regression fixture produced six entries instead of two before
the correction. The fix compares the complete relative directory path.

An optimized probe linked to `compass-xdg` was run inside the existing
`com.vicinae.Vicinae` Flatpak sandbox on the developer's GNOME host. Only the
temporary executable directory was additionally exposed read-only. The probe
called `find_icon(name, Some(default_theme()), Some(32), 1.0)` and read the returned
file, timing those two operations together. The selected theme was Adwaita.

Before source: e6200d2cd15f3b46c203e70e34e28454e41b3437. After: the directory-path
comparison correction in `codex/icon-theme-directory-parser`, based on that head.
Both library builds used Rust 1.94.1, `cargo build --locked --release -j 1 -p
compass-xdg`; the probe used `rustc -O`.

| Icon name | Before | After | Resolved format |
|---|---:|---:|---|
| io.github.kolunmi.Bazaar | 258.750 ms | 8.186 ms | SVG |
| org.chromium.Chromium | 251.428 ms | 8.024 ms | PNG |
| org.gnome.baobab | 103.102 ms | 3.877 ms | SVG |
| org.gnome.DiskUtility | 102.238 ms | 3.598 ms | SVG |

These are single exploratory samples with a warm filesystem, not a statistical
benchmark, cold-start measurement, or upstream comparison. No general speedup or
memory-efficiency claim follows. They identify a concrete regression to fix before
the icons-on default is installed. First-time lookup still performs filesystem
work synchronously; asynchronous loading/caching remains an optimization target.

Tests cover grouped metadata for nested directories and selecting the closest
sufficient fixed-size icon even when a larger directory appears first. Existing
XDG tests remain required; icon theme lookup is not claimed fully upstream-parity
complete by this targeted correction.
