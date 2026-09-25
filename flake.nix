{
  description = "A focused launcher for your desktop - native, fast, extensible";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs?ref=nixos-unstable";
    systems.url = "github:nix-systems/default";
    crane = {
      url = "github:ipetkov/crane/v0.17.0";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  nixConfig = {
    extra-substituters = ["https://vicinae.cachix.org"];
    extra-trusted-public-keys = [
      "vicinae.cachix.org-1:1kDrfienkGHPYbkpNj1mWTr7Fm1+zcenzgTizIcI3oc="
    ];
  };

  outputs = {
    self,
    nixpkgs,
    systems,
    crane,
  }: let
    inherit (nixpkgs) lib;

    forEachPkgs = f: lib.genAttrs (import systems) (system: f nixpkgs.legacyPackages.${system});

    # The engine is Rust and Linux only (ADR-0007, ADR-0021).
    forEachLinuxPkgs = f:
      lib.genAttrs (lib.filter (lib.hasSuffix "-linux") (import systems))
      (system: f nixpkgs.legacyPackages.${system});

    # Built with crane and installed the way the Flatpak installs it. See
    # packaging/nix/compass.nix.
    rustBuild = pkgs: let
      src = lib.cleanSource self;
    in rec {
      extension-runtime = pkgs.callPackage ./packaging/nix/extension-runtime.nix {inherit src;};
      compass = pkgs.callPackage ./packaging/nix/compass.nix {
        craneLib = crane.mkLib pkgs;
        inherit src;
        extensionRuntime = extension-runtime;
      };
    };
  in {
    packages = forEachLinuxPkgs (
      pkgs: let
        built = rustBuild pkgs;
      in {
        inherit (built) compass extension-runtime;
        default = built.compass;
        rust-vicinae = built.compass;
        nix-update-script = pkgs.writeShellScriptBin "nix-update-script" ''
          OLD_API_DEPS_HASH=$(${pkgs.lib.getExe pkgs.nix} eval --raw .#packages.x86_64-linux.extension-runtime.apiDeps.hash)
          OLD_EXT_MAN_DEPS_HASH=$(${pkgs.lib.getExe pkgs.nix} eval --raw .#packages.x86_64-linux.extension-runtime.extensionManagerDeps.hash)

          cd src/typescript/api
          NEW_API_DEPS_HASH=$(${pkgs.lib.getExe pkgs.prefetch-npm-deps} package-lock.json)
          cd ../extension-manager
          NEW_EXT_MAN_DEPS_HASH=$(${pkgs.lib.getExe pkgs.prefetch-npm-deps} package-lock.json)
          cd ..

          [[ "$OLD_API_DEPS_HASH" == "$NEW_API_DEPS_HASH" ]] || { echo -e "\e[31mHash mismatch for API npm deps, please replace the value in packaging/nix/extension-runtime.nix with '$NEW_API_DEPS_HASH'.\e[0m" >&2; exit 1; }

          [[ "$OLD_EXT_MAN_DEPS_HASH" == "$NEW_EXT_MAN_DEPS_HASH" ]] || { echo -e "\e[31mHash mismatch for extension-manager npm deps, please replace the value in packaging/nix/extension-runtime.nix with '$NEW_EXT_MAN_DEPS_HASH'.\e[0m" >&2; exit 1; }
        '';
      }
    );

    lib = forEachPkgs (pkgs: {
      mkVicinaeExtension = pkgs.callPackage ./nix/mkVicinaeExtension.nix {};
      mkRayCastExtension = pkgs.callPackage ./nix/mkRayCastExtension.nix {};
    });

    devShells = forEachLinuxPkgs (pkgs: {
      default = pkgs.mkShell {
        # automatically pulls nativeBuildInputs + buildInputs
        inputsFrom = [self.packages.${pkgs.stdenv.hostPlatform.system}.compass];
        packages = with pkgs; [
          cargo
          clippy
          rustfmt
          nodejs
        ];
      };
    });

    overlays.default = final: prev: {
      compass = self.packages.${final.stdenv.hostPlatform.system}.compass;
      vicinae = self.packages.${final.stdenv.hostPlatform.system}.compass;
      mkVicinaeExtension = prev.callPackage ./nix/mkVicinaeExtension.nix {};
      mkRayCastExtension = prev.callPackage ./nix/mkRayCastExtension.nix {};
    };

    homeManagerModules.default = import ./nix/home-manager-module.nix self;
    nixosModules.default = import ./nix/nixos-module.nix self;
  };
}
