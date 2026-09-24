#!/usr/bin/env bash
# Builds the extension runtime bundle: the thing the C++ engine ships as
# vicinae-worker-ts, and the thing `crates/compass-worker-host`'s real_runtime
# test drives.
#
# Three steps, none of which cargo can do:
#
#   1. build figura, the IDL compiler, from src/lib/figura. It needs only
#      vicinae::common -- no Qt -- so this configures it standalone rather than
#      building the whole C++ engine;
#   2. generate the TypeScript protos from figura/*.fig, for both the API
#      package and the extension manager;
#   3. npm install and bundle with the manager's own esbuild script.
#
# Usage: scripts/build-extension-runtime.sh
# Output: src/typescript/extension-manager/dist/runtime.js
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
build_dir="${BUILD_DIR:-$repo_root/.build/extension-runtime}"
api_dir="$repo_root/src/typescript/api"
mgr_dir="$repo_root/src/typescript/extension-manager"

# std::ranges::to, which figura's parser uses, needs a C++23 standard library:
# GCC 14 or newer. CXX may point at anything that has one.
cxx="${CXX:-}"
if [ -z "$cxx" ]; then
  for candidate in g++-15 g++-14 g++; do
    if command -v "$candidate" >/dev/null 2>&1; then
      cxx="$candidate"
      break
    fi
  done
fi

mkdir -p "$build_dir"
cat > "$build_dir/CMakeLists.txt" <<CMAKE
cmake_minimum_required(VERSION 3.16)
project(figura-standalone CXX)
set(CMAKE_CXX_STANDARD 23)
set(VICINAE_LIBEXEC_DIR "/usr/libexec")
set(VICINAE_LIBEXEC_PATH "/usr/libexec")
include_directories($repo_root/vendor)
add_subdirectory($repo_root/src/lib/common common-build)
add_subdirectory($repo_root/src/lib/figura figura-build)
CMAKE

echo "building figura with ${cxx}"
cmake -S "$build_dir" -B "$build_dir/build" -DCMAKE_CXX_COMPILER="$cxx" >/dev/null
cmake --build "$build_dir/build" -j"$(nproc)" >/dev/null
figura="$build_dir/build/figura-build/figura"

echo "generating protos"
mkdir -p "$mgr_dir/src/proto" "$api_dir/src/api/proto" "$api_dir/src/proto"
"$figura" compile "$repo_root/figura/manager.fig" --server typescript \
  --output "$mgr_dir/src/proto/manager.ts"
"$figura" compile "$repo_root/figura/manager-extension.fig" --client typescript \
  --output "$mgr_dir/src/proto/manager-extension.ts"
"$figura" compile "$repo_root/figura/manager-extension.fig" --server typescript \
  --output "$mgr_dir/src/proto/extension-manager.ts"
"$figura" compile "$repo_root/figura/tsapi.fig" --client typescript \
  --output "$mgr_dir/src/proto/api.ts"
"$figura" compile "$repo_root/figura/tsapi.fig" --client typescript \
  --output "$api_dir/src/api/proto/api.ts"
"$figura" compile "$repo_root/figura/ipc.fig" --client typescript \
  --output "$api_dir/src/proto/ipc.ts"

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
