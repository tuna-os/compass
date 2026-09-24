# The extension runtime bundle (extension-runtime.js) for the Rust engine.
#
# Built by the same scripts/build-extension-runtime.sh the Flatpak workflow and
# the Arch package run: figura (C++23) generates the protocol, npm bundles it.
# The npm dependencies are the C++ package's own fetchNpmDeps outputs, passed
# in by flake.nix, so their hashes live in one place (nix/vicinae.nix) and
# `nix run .#nix-update-script` keeps checking them.
{
  lib,
  gcc15Stdenv,
  cmake,
  nodejs,
  npmHooks,
  src,
  apiDeps,
  extensionManagerDeps,
}:
gcc15Stdenv.mkDerivation {
  pname = "compass-extension-runtime";
  version = "0.1.0";
  inherit src;

  strictDeps = true;
  nativeBuildInputs = [
    cmake
    nodejs
  ];
  # cmake is for figura, which the script configures itself.
  dontUseCmakeConfigure = true;

  postPatch = ''
    local postPatchHooks=()
    source ${npmHooks.npmConfigHook}/nix-support/setup-hook
    npmRoot=src/typescript/api npmDeps=${apiDeps} npmConfigHook
    npmRoot=src/typescript/extension-manager npmDeps=${extensionManagerDeps} npmConfigHook
  '';

  buildPhase = ''
    runHook preBuild
    # node_modules are already installed offline by npmConfigHook above.
    SKIP_NPM_INSTALL=1 BUILD_DIR="$TMPDIR/extension-runtime" \
      bash scripts/build-extension-runtime.sh
    runHook postBuild
  '';

  installPhase = ''
    runHook preInstall
    install -Dm644 src/typescript/extension-manager/dist/runtime.js \
      "$out/share/vicinae/extension-runtime.js"
    runHook postInstall
  '';

  meta = {
    description = "Extension runtime bundle for the Compass (Rust) engine";
    license = lib.licenses.gpl3Only;
    platforms = lib.platforms.linux;
  };
}
