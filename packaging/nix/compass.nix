# The Rust engine, installed the way the Flatpak installs it.
#
# `nix build .#rust-vicinae` (alias `.#compass`). The layout comes from
# scripts/packaging/install-rust-engine.sh, shared with the AppImage and the
# Arch package, and scripts/packaging/smoke.sh checks it in CI (Suite 5).
#
# The whole repository is the source, not a filter of Cargo files: the build
# reads SQL migrations, the extension boilerplate, vendor/fuzzy-trigram's C and
# the C++ glyph and icon tables through include_str! and build.rs, and a
# Cargo-only filter silently dropped them.
{
  lib,
  craneLib,
  src,
  extensionRuntime,
  pkg-config,
  makeWrapper,
  patchelf,
  nodejs,
  openssl,
  wayland,
  libxkbcommon,
  vulkan-loader,
  libGL,
}: let
  # winit and wgpu dlopen these rather than linking them, so fixup's
  # --shrink-rpath would strip them; they are added back after it, below.
  dlopened = lib.makeLibraryPath [
    wayland
    libxkbcommon
    vulkan-loader
    libGL
  ];
in
  craneLib.buildPackage {
    pname = "compass";
    version = "0.1.0";
    inherit src;

    strictDeps = true;
    cargoExtraArgs = "--locked -p vicinae -p compass-sandbox --bins";
    doCheck = false;

    nativeBuildInputs = [
      pkg-config
      makeWrapper
      patchelf
    ];
    # libsqlite3-sys's bundled SQLCipher links libcrypto; iced_layershell's
    # smithay-client-toolkit needs xkbcommon at build time.
    buildInputs = [
      openssl
      libxkbcommon
    ];
    OPENSSL_LIB_DIR = "${lib.getLib openssl}/lib";
    OPENSSL_INCLUDE_DIR = "${lib.getDev openssl}/include";

    installPhaseCommand = ''
      PREFIX="$out" LIBEXECDIR="$out/libexec" BIN_DIR="$PWD/target/release" \
        RUNTIME_JS=${extensionRuntime}/share/vicinae/extension-runtime.js REQUIRE_RUNTIME=1 \
        bash scripts/packaging/install-rust-engine.sh
    '';

    # COMPASS_NODE rather than Node on PATH: the launcher passes its
    # environment to every application it starts, and they should not inherit
    # our Node. COMPASS_BUILTIN_ICONS because `nix run` puts nothing on
    # XDG_DATA_DIRS. The helpers, the runtime bundle and the schema are found
    # relative to the binary, which the wrapper keeps in $out/bin.
    postFixup = ''
      patchelf --add-rpath ${dlopened} "$out/bin/vicinae"
      wrapProgram "$out/bin/vicinae" \
        --set-default COMPASS_NODE ${lib.getExe nodejs} \
        --set-default COMPASS_BUILTIN_ICONS "$out/share/vicinae/builtin-icons"
    '';

    meta = {
      description = "Compass, the Rust engine of Vicinae";
      homepage = "https://github.com/tuna-os/compass";
      license = lib.licenses.gpl3Only;
      platforms = lib.platforms.linux;
      mainProgram = "vicinae";
    };
  }
