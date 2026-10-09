#! /bin/bash

set -euo pipefail

# Atomically bump the project version:
#   1. write the new tag + the commit it is based on into the manifest
#   2. move every version source to the tag (workspace Cargo.toml, Nix, the
#      Flatpak metainfo, the Arch pkgver prefix, the Homebrew formula's url):
#      `compass --version` reports CARGO_PKG_VERSION, and
#      crates/compass/tests/version_sync.rs fails while it disagrees with
#      the manifest tag
#   3. commit the version changes
#
# Open a PR with that commit. When it merges, release.yml sees the new tag in
# manifest.yaml, tags the merge commit and publishes the release, so the tag
# always lands on a commit that carries the updated manifest.

bump_version() {
    local version_type=${1:-patch}
    local manifest=${2:-./manifest.yaml}

    if ! git diff --cached --quiet; then
        echo "refusing to bump: the index has staged changes, commit or unstage them first" >&2
        exit 1
    fi

    # The manifest names the last release, so it is the source of truth — not
    # the tag list. Tags may be missing from a fresh clone (or were never
    # pushed), and deriving from them then recomputes a version that already
    # exists: with no tags at all, `v0.0.0` + patch yields `v0.0.1` on top of
    # a `v0.28.1` manifest. Git tags are only the fallback for a manifest
    # that names nothing yet.
    local current_tag
    current_tag=$(sed -n 's/^  tag: "\(.*\)"$/\1/p' "$manifest")
    if [ -z "$current_tag" ]; then
        current_tag=$(git tag -l 'v*' --sort=-v:refname | head -n1)
    fi
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
    short_rev=$(git rev-parse --short=9 HEAD)

    sed -i -e "s/^  tag: \".*\"$/  tag: \"${new_version}\"/" \
        -e "s/^  rev: \".*\"$/  rev: \"${rev}\"/" \
        -e "s/^  short_rev: \".*\"$/  short_rev: \"${short_rev}\"/" "$manifest"

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
    # AppStream's <releases> is a history, newest first: add the new release
    # above the others rather than renaming the last one.
    sed -i "0,/^\(\s*\)<releases>\$/s||&\n\1  <release version=\"${bare}\" date=\"$(date +%F)\"/>|" \
        "$repo_root/packaging/flatpak/org.tunaos.compass.metainfo.xml"
    sed -i "s/^pkgver=.*/pkgver=${bare}.r0.g0000000/" "$repo_root/packaging/arch/PKGBUILD"
    sed -i "s/printf '[0-9.]*\.r%s\.g%s'/printf '${bare}.r%s.g%s'/" \
        "$repo_root/packaging/arch/PKGBUILD"
    # The in-tree Homebrew formula tracks the tag; its sha256 cannot follow
    # (the tarball exists only after the tag is pushed), so the bump resets
    # it to the placeholder and filling it is part of the release. A stale
    # hash would fail the install; the placeholder fails the audit first.
    sed -i "s|/tags/v[0-9.]*\.tar\.gz|/tags/${new_version}.tar.gz|" \
        "$repo_root/packaging/homebrew/compass.rb"
    sed -i 's/^\s*sha256 "[0-9a-f]\{64\}"$/  sha256 "REPLACE_WITH_RELEASE_TARBALL_SHA256"/' \
        "$repo_root/packaging/homebrew/compass.rb"
    # Refreshing the lock needs the registry: a fresh clone's cache misses
    # crates for targets it never built (e.g. android-activity), and --offline
    # fails there. A release pushes right after, so network is assumed; a
    # missing cargo is still only a warning, and --locked builds (Flatpak,
    # Homebrew) fail loudly on a stale lock rather than shipping it.
    if command -v cargo >/dev/null 2>&1; then
        (cd "$repo_root" && cargo metadata --format-version=1 >/dev/null)
    else
        echo "warning: no cargo; Cargo.lock still names the old version" >&2
    fi

    git add "$manifest" Cargo.toml Cargo.lock \
        packaging/nix/compass.nix packaging/nix/extension-runtime.nix \
        packaging/flatpak/org.tunaos.compass.metainfo.xml \
        packaging/arch/PKGBUILD packaging/homebrew/compass.rb
    git commit -m "chore: bump to ${new_version}"

    echo "bumped to ${new_version}: open a PR with this commit; merging it releases ${new_version}"
}

bump_version "$@"
