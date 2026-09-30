#!/usr/bin/env bash
# Phase 4's third gate condition (PLAN §6): `compass-extension-api` compiles and
# passes its tests with `compass-worker-host` removed from the dependency graph.
#
# Two checks, because either alone can be fooled:
#
#   1. In the real workspace, `cargo tree` for the crate — normal, build and
#      dev edges — must not reach compass-worker-host. A feature or a
#      dev-dependency added later would show up here first.
#   2. The crate is copied into a workspace of its own in which
#      compass-worker-host does not exist at all — not unused, absent — and
#      built and tested there, against the same Cargo.lock pins. A dependency
#      the tree check missed (a path dependency written straight into the
#      crate's Cargo.toml, say) cannot resolve there, so the build fails.
#
# Usage: scripts/ci/extension-api-seam.sh
# Honours CARGO_TARGET_DIR for step 2; defaults to a directory under the
# temporary workspace, removed on exit.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
crate=compass-extension-api
forbidden=compass-worker-host

echo "1. the dependency graph, in the real workspace"
tree="$(cd "$repo_root" && cargo tree -p "$crate" -e normal,build,dev --prefix none --format '{p}')"
if grep -q "^$forbidden " <<<"$tree"; then
  echo "::error::$crate depends on $forbidden:"
  cd "$repo_root" && cargo tree -p "$crate" -e normal,build,dev -i "$forbidden"
  exit 1
fi
others="$(grep -o '^compass-[a-z0-9-]*' <<<"$tree" | sort -u | grep -vx "$crate" || true)"
echo "   no $forbidden; other Compass crates reached: ${others:-none}"

echo "2. the crate alone, in a workspace without $forbidden"
scratch="$(mktemp -d)"
trap 'rm -rf "$scratch"' EXIT
mkdir -p "$scratch/crates"
cp -r "$repo_root/crates/$crate" "$scratch/crates/$crate"
cp "$repo_root/Cargo.lock" "$repo_root/rust-toolchain.toml" "$scratch/"
# The root manifest's package metadata, dependency versions and lints, with
# every in-tree path dependency dropped and the member list narrowed to one.
sed -e '/path = "crates\//d' \
    -e 's|^members = .*|members = ["crates/'"$crate"'"]|' \
    "$repo_root/Cargo.toml" > "$scratch/Cargo.toml"
if [ -e "$scratch/crates/$forbidden" ] || grep -q "$forbidden" "$scratch/Cargo.toml"; then
  echo "::error::the isolated workspace still names $forbidden"
  exit 1
fi
(
  cd "$scratch"
  export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$scratch/target}"
  if cargo metadata --format-version 1 | grep -q "\"name\":\"$forbidden\""; then
    echo "::error::$forbidden resolved in a workspace that does not contain it"
    exit 1
  fi
  cargo test -p "$crate"
)
echo "$crate builds and passes its tests without $forbidden"
