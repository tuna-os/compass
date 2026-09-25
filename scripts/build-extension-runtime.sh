#!/usr/bin/env bash
# Builds the extension runtime bundle: the thing the Rust engine installs as
# extension-runtime.js, and the thing `crates/compass-worker-host`'s
# real_runtime test drives.
#
# npm installs and the manager's own esbuild script bundles. The protocol
# bindings under src/proto are committed: `make figen` regenerates them from
# figura/*.fig, and compass-figura's test fails when they are stale.
#
# Usage: scripts/build-extension-runtime.sh
# Output: src/typescript/extension-manager/dist/runtime.js
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
api_dir="$repo_root/src/typescript/api"
mgr_dir="$repo_root/src/typescript/extension-manager"

# `npm ci` rather than `npm install`: it installs exactly what the lockfile
# says and, unlike `install`, never rewrites it -- building the runtime must not
# produce a diff.
#
# SKIP_NPM_INSTALL=1 is for builds that install node_modules themselves,
# offline: the Nix package does it with npmConfigHook.
if [ "${SKIP_NPM_INSTALL:-0}" != 1 ]; then
  echo "installing node modules"
  (cd "$api_dir" && npm ci --no-audit --no-fund --silent)
  (cd "$mgr_dir" && npm ci --no-audit --no-fund --silent)
fi

echo "bundling the runtime"
(cd "$mgr_dir" && npm run build-only)

echo "runtime at $mgr_dir/dist/runtime.js"
