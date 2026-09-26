#! /bin/bash

set -euo pipefail

# Atomically bump the project version:
#   1. write the new tag + the commit it is based on into the manifest
#   2. move every version source to the tag (workspace Cargo.toml, Nix, the
#      Flatpak metainfo, the Arch pkgver prefix): `compass --version` reports
#      CARGO_PKG_VERSION, and crates/compass/tests/version_sync.rs fails while
#      it disagrees with the manifest tag
#   3. commit the version changes
#   4. tag *that* commit
#
# The tag must land on the commit that carries the updated manifest, otherwise
# anyone checking out the tag gets a manifest pointing at the previous release.

bump_version() {
    local version_type=${1:-patch}
    local manifest=${2:-./manifest.yaml}

    if ! git diff --cached --quiet; then
        echo "refusing to bump: the index has staged changes, commit or unstage them first" >&2
        exit 1
    fi

    # Use the highest version tag across the whole repo, not just tags reachable
    # from HEAD: bump commits are tagged but never merged back into the working
    # branch, so `git describe` would miss the most recent release and we'd
    # recompute a version that already exists.
    local current_tag
    current_tag=$(git tag -l 'v*' --sort=-v:refname | head -n1)
    current_tag=${current_tag:-v0.0.0}

    local current_version=${current_tag#v}
    IFS='.' read -ra VERSION_PARTS <<< "$current_version"
    local major=${VERSION_PARTS[0]:-0}
    local minor=${VERSION_PARTS[1]:-0}
    local patch=${VERSION_PARTS[2]:-0}

    case $version_type in
        major) major=$((major + 1)); minor=0; patch=0 ;;
        minor) minor=$((minor + 1)); patch=0 ;;
        patch) patch=$((patch + 1)) ;;
        *) echo "unknown version type: ${version_type} (expected major|minor|patch)" >&2; exit 1 ;;
    esac

    local new_version="v$major.$minor.$patch"

    # The release is built from the current HEAD: record it before we create the
    # bump commit so the manifest references the actual code commit.
    local rev short_rev
    rev=$(git rev-parse HEAD)
    short_rev=$(git rev-parse --short HEAD)

    yq -i ".release.tag = \"${new_version}\" | .release.rev = \"${rev}\" | .release.short_rev = \"${short_rev}\"" "$manifest"

    # The tag without its `v`: the form every version source carries.
    local bare=${new_version#v}
    local repo_root
    repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
    # Workspace Cargo.toml: the `^version` anchor is the [workspace.package]
    # line alone; dependency versions are indented.
    sed -i "s/^version = \".*\"/version = \"${bare}\"/" "$repo_root/Cargo.toml"
    sed -i "s/^\(\s*\)version = \"[^\"]*\";/\1version = \"${bare}\";/" \
        "$repo_root/packaging/nix/compass.nix" \
        "$repo_root/packaging/nix/extension-runtime.nix"
    sed -i "s|<release version=\"[^\"]*\" date=\"[^\"]*\"|<release version=\"${bare}\" date=\"$(date +%F)\"|" \
        "$repo_root/packaging/flatpak/org.tunaos.compass.metainfo.xml"
    sed -i "s/^pkgver=.*/pkgver=${bare}.r0.g0000000/" "$repo_root/packaging/arch/PKGBUILD"
    sed -i "s/printf '[0-9.]*\.r%s\.g%s'/printf '${bare}.r%s.g%s'/" \
        "$repo_root/packaging/arch/PKGBUILD"
    if command -v cargo >/dev/null 2>&1; then
        (cd "$repo_root" && cargo metadata --format-version=1 --offline >/dev/null)
    fi

    git add "$manifest" Cargo.toml Cargo.lock \
        packaging/nix/compass.nix packaging/nix/extension-runtime.nix \
        packaging/flatpak/org.tunaos.compass.metainfo.xml \
        packaging/arch/PKGBUILD
    git commit -m "chore: bump to ${new_version}"
    git tag "${new_version}"

    echo "bumped to ${new_version} (tag on $(git rev-parse --short HEAD))"
}

bump_version "$@"
