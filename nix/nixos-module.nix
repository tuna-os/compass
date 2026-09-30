self: {
  pkgs,
  lib,
  config,
  ...
}: let
  cfg = config.programs.compass.input-server;
  wrapper = "${config.security.wrapperDir}/compass-input-server";
in {
  imports = [
    (lib.mkRenamedOptionModule ["programs" "vicinae" "input-server" "enable"] ["programs" "compass" "input-server" "enable"])
    (lib.mkRenamedOptionModule ["programs" "vicinae" "input-server" "package"] ["programs" "compass" "input-server" "package"])
  ];

  options.programs.compass.input-server = {
    enable = lib.mkOption {
      type = lib.types.bool;
      default = true;
      description = ''
        Install Compass's input server with the capability it needs to read `/dev/input` and
        write `/dev/uinput`. Snippet keywords do not expand without it.
      '';
    };
    package = lib.mkPackageOption self.packages.${pkgs.stdenv.hostPlatform.system} "compass" {
      extraDescription = "The Compass package whose input server is wrapped.";
    };
  };

  # The Nix store cannot hold file capabilities, so the helper runs from a
  # wrapper, and COMPASS_INPUT_SERVER_BIN points the engine at it
  # (packaging/README.md, "The input server").
  config = lib.mkIf cfg.enable {
    security.wrappers.compass-input-server = {
      source = "${cfg.package}/libexec/compass/compass-input-server";
      capabilities = "cap_dac_override+ep";
      owner = "root";
      group = "root";
    };

    environment.sessionVariables.COMPASS_INPUT_SERVER_BIN = wrapper;
  };
}
