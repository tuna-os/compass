{pkgs ? import <nixpkgs> {}}: {
  vicinae = pkgs.callPackage ./nix/compass.nix {};
}
