#!/usr/bin/env bash
# Assert the headers and libraries the C++ tree consumes DIRECTLY are present.
#
# WHY THIS EXISTS
#
# The configure job cannot catch these. `cmake` only resolves what the tree asks
# for through find_package, and this tree also consumes dependencies two ways
# CMake never checks:
#
#   - bare link names, e.g. src/server/CMakeLists.txt:990
#       list(APPEND LIBS xkbcommon xkbcommon-x11)
#     which become -lxkbcommon at link time and are not looked for at all
#     until then;
#   - bare #includes of system headers with no find_package behind them.
#
# So a missing libxkbcommon-devel configures perfectly and then fails at object
# 624 of 812, eighteen minutes in. That is exactly what happened, and adding one
# package per red run is the pattern that already cost four cycles on the
# configure job. This checks the whole set in about a second.
#
# HOW THE LIST WAS DERIVED, so it can be re-derived rather than guessed:
#
#   headers: grep the tree for namespaced system #includes, drop Qt, KF6, the
#            vendored trees (xdgpp, sqlcipher, zip, pugixml, cmark-gfm), the
#            Windows ones (winrt/, wrl/) and Catch2, which the Bluefin builds
#            disable.
#   libs:    grep target_link_libraries for names that are neither CMake targets
#            (they contain no ::) nor vendored.
#
# Qt itself is deliberately absent: configure already proves every Qt component
# resolves, and probing Qt headers here would need the include paths CMake
# computes.
set -euo pipefail

status=0

check_header() {
    if echo "#include <$1>" | c++ -fsyntax-only -x c++ - 2>/dev/null; then
        echo "  header ok      $1"
    else
        echo "  header MISSING $1"
        status=1
    fi
}

check_lib() {
    if echo 'int main(){return 0;}' | c++ -x c++ - "-l$1" -o /dev/null 2>/dev/null; then
        echo "  lib    ok      -l$1"
    else
        echo "  lib    MISSING -l$1"
        status=1
    fi
}

echo "Headers the tree includes directly:"
check_header xkbcommon/xkbcommon.h
check_header xkbcommon/xkbcommon-keysyms.h
check_header xkbcommon/xkbcommon-x11.h
check_header libudev.h
check_header wayland-client.h
check_header xcb/xcb.h
check_header xcb/xproto.h
check_header xcb/xcb_keysyms.h
check_header xcb/xkb.h
check_header openssl/evp.h
check_header openssl/kdf.h
check_header openssl/rand.h
check_header libqalculate/Calculator.h

echo "Libraries the tree links by bare name:"
check_lib xkbcommon
check_lib xkbcommon-x11
check_lib udev
check_lib wayland-client

if [ "$status" -ne 0 ]; then
    echo "::error::a dependency the tree uses directly is not installed — see above"
fi
exit "$status"
