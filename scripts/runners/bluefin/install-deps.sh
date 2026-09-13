#!/usr/bin/env bash
# Build dependencies for the C++ engine on the Bluefin target.
#
# Single source of truth, for the same reason the Arch script next door is one:
# .github/workflows/cpp-on-target.yaml runs configure and the full build as two
# jobs, and a list duplicated between them is a list that drifts.
#
# This is Arch's list translated, and the translation is not one-to-one:
#
#   qt6-qtbase-private-devel   Arch ships private headers inside qt6-base;
#                              Fedora splits them out, and src/server links
#                              Qt6::GuiPrivate.
#   layer-shell-qt-devel       Required, not optional. USE_SYSTEM_LAYER_SHELL
#                              is ON unconditionally (CMakeLists.txt:53) — the
#                              FetchContent path is reached only under
#                              PREFER_STATIC_LIBS, which is the AppImage build.
#   libsecret-devel            Fedora has no Qt6 qtkeychain, so the build uses
#                              -DUSE_SYSTEM_QT_KEYCHAIN=OFF and the vendored
#                              copy still links system libsecret. The option's
#                              own help text says so.
#   no catch2                  Fedora 44 ships 2.13.10; the tests require
#                              Catch2 3, which Fedora has no package for. The
#                              builds here pass -DBUILD_TESTS=OFF, and that is
#                              the right answer rather than a workaround: Suite
#                              0 diffs engine behaviour through the CLI, and the
#                              C++ unit tests already run on Arch where Catch2
#                              is v3.
#
# The rest was enumerated from the tree's own find_package calls — ECM,
# KF6SyntaxHighlighting, LayerShellQt, OpenSSL, PkgConfig and X11 REQUIRED with
# the xcb, xcb_keysyms and xcb_xkb components — rather than discovered one red
# CI run at a time, which is how the first four revisions of this list were
# found and is a poor way to spend cycles even cheap ones.
set -euo pipefail

dnf install -y --setopt=install_weak_deps=False \
    gcc gcc-c++ cmake ninja-build git \
    qt6-qtbase-devel qt6-qtbase-private-devel \
    qt6-qtsvg-devel qt6-qtdeclarative-devel \
    qt6-qtshadertools-devel qt6-qttools-devel \
    qt6-qtwayland-devel \
    kf6-syntax-highlighting-devel extra-cmake-modules \
    layer-shell-qt-devel \
    libqalculate-devel \
    wayland-devel wayland-protocols-devel \
    libsecret-devel openssl-devel \
    libX11-devel libxcb-devel xcb-util-keysyms-devel \
    nodejs npm
