#!/usr/bin/env bash
# Installs a built Rust engine into a prefix, laid out the way the Flatpak lays
# out /app. The AppImage, the Arch package and the Nix package all call this,
# so "what a Compass install contains" has one definition outside the Flatpak
# manifest -- and packaging/check-install.sh asserts the same list.
#
# Usage:
#   DESTDIR=/tmp/root PREFIX=/usr LIBEXECDIR=/usr/lib \
#     scripts/packaging/install-rust-engine.sh
#
# Environment:
#   PREFIX       install prefix (default /usr/local)
#   LIBEXECDIR   internal programs go in $LIBEXECDIR/compass (default
#                $PREFIX/libexec). Must be $PREFIX/libexec or $PREFIX/lib:
#                the engine looks for its helpers relative to its own binary
#                (crates/compass/src/indexer_client.rs helper_candidates).
#   DESTDIR      staging root, prepended to every path (default empty)
#   BIN_DIR      where cargo put the release binaries (default target/release)
#   RUNTIME_JS   the extension runtime bundle (default
#                src/typescript/extension-manager/dist/runtime.js). Missing is
#                a warning, as in the Flatpak, unless REQUIRE_RUNTIME=1.
#   NODE_BIN     a Node.js binary to ship as $PREFIX/bin/node (AppImage only;
#                distribution packages depend on their own nodejs instead)
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

prefix="${PREFIX:-/usr/local}"
libexecdir="${LIBEXECDIR:-$prefix/libexec}"
destdir="${DESTDIR:-}"
bin_dir="${BIN_DIR:-$repo_root/target/release}"
runtime_js="${RUNTIME_JS:-$repo_root/src/typescript/extension-manager/dist/runtime.js}"

case "$libexecdir" in
  "$prefix/libexec" | "$prefix/lib") ;;
  *)
    echo "LIBEXECDIR must be \$PREFIX/libexec or \$PREFIX/lib, got $libexecdir" >&2
    exit 1
    ;;
esac

root="$destdir$prefix"
helpers="$destdir$libexecdir/compass"
share="$root/share"
app_id=org.tunaos.compass

for bin in compass compass-file-indexer compass-sandbox-exec; do
  if [ ! -x "$bin_dir/$bin" ]; then
    echo "missing $bin_dir/$bin; build with:" >&2
    echo "  cargo build --release --locked -p compass -p compass-sandbox -p compass-input-server --bins" >&2
    exit 1
  fi
done

install -Dm755 "$bin_dir/compass" "$root/bin/compass"
# Both helpers sit where the engine searches: ../libexec/compass or
# ../lib/compass from bin/. compass-sandbox-exec confines the extension runtime
# (Landlock + seccomp) and the engine refuses to run extensions without it.
install -Dm755 "$bin_dir/compass-file-indexer" "$helpers/compass-file-indexer"
install -Dm755 "$bin_dir/compass-sandbox-exec" "$helpers/compass-sandbox-exec"

# The snippet keyword expander's keyboard helper (crates/compass-input-server).
# Optional: the Flatpak cannot use it (no /dev/input in the sandbox), so it is
# built only where it can run (-p compass-input-server). It needs
# cap_dac_override to read /dev/input and write /dev/uinput, which DESTDIR
# staging cannot carry: the package grants it at install time (Arch:
# compass.install; NixOS: security.wrappers), as `make postbuild` does for the
# C++ helper. See packaging/README.md, "The input server".
if [ -x "$bin_dir/compass-input-server" ]; then
  install -Dm755 "$bin_dir/compass-input-server" "$helpers/compass-input-server"
else
  echo "warning: no $bin_dir/compass-input-server; snippet keywords will not expand" >&2
fi

install -Dm644 "$repo_root/packaging/flatpak/$app_id.desktop" \
  "$share/applications/$app_id.desktop"
install -Dm644 "$repo_root/packaging/flatpak/$app_id.metainfo.xml" \
  "$share/metainfo/$app_id.metainfo.xml"
install -Dm644 "$repo_root/extra/compass.svg" \
  "$share/icons/hicolor/scalable/apps/$app_id.svg"

# An opt-in user unit (`systemctl --user enable --now compass`). Flatpak does
# not export units, so the copy in /app is inert.
install -Dm644 "$repo_root/packaging/systemd/compass.service" \
  "$root/lib/systemd/user/compass.service"

# The Icon.* set extensions draw with, found through $XDG_DATA_DIRS as
# compass/builtin-icons (compass_core::builtin_icon).
install -Dm644 -t "$share/compass/builtin-icons" "$repo_root"/src/server/icons/*.svg

# The published compass.json schema, so an editor can be pointed at a local
# copy that matches the installed build.
install -Dm644 "$repo_root/packaging/schema/compass.schema.json" \
  "$share/compass/compass.schema.json"

# The first-party Rhai scripts, found through $XDG_DATA_DIRS as compass/scripts
# and at ../share/compass/scripts from bin/ (crates/compass/src/rhai_scripts.rs).
# Packaged scripts are granted what their manifests declare; the user's own are
# asked about first (docs/rust-engine/RHAI-SCRIPTS.md, "Permissions").
for script in "$repo_root"/extensions/rhai-examples/*/; do
  name="$(basename "$script")"
  install -Dm644 -t "$share/compass/scripts/$name" "$script"script.toml "$script"*.rhai
done

# The extension runtime bundle, found at ../share/compass/ from bin/.
if [ -f "$runtime_js" ]; then
  install -Dm644 "$runtime_js" "$share/compass/extension-runtime.js"
elif [ "${REQUIRE_RUNTIME:-0}" = 1 ]; then
  echo "no extension runtime bundle at $runtime_js; run scripts/build-extension-runtime.sh" >&2
  exit 1
else
  echo "warning: no extension runtime bundle at $runtime_js; extensions will not run" >&2
fi

if [ -n "${NODE_BIN:-}" ]; then
  install -Dm755 "$NODE_BIN" "$root/bin/node"
fi

echo "installed the Rust engine into $root (helpers in $helpers)"
