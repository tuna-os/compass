self: {
  config,
  pkgs,
  lib,
  ...
}: let
  cfg = config.programs.compass;

  jsonFormat = pkgs.formats.json {};
  tomlFormat = pkgs.formats.toml {};

  envVarType = with lib.types; attrsOf (oneOf [str int float bool]);

  envValueToString = val:
    if lib.isBool val
    then
      (
        if val
        then "1"
        else "0"
      )
    else toString val;

  # Upstream Vicinae's option names, kept so an existing configuration still
  # evaluates after switching to this flake.
  renamedOptions = [
    "enable"
    "package"
    "extensions"
    "themes"
    "settings"
  ];
  renamedSystemdOptions = [
    "enable"
    "autoStart"
    "environment"
    "target"
  ];

  # Options with no Compass equivalent. The calculator is fend-core, the
  # browser bridge is out of scope (ADR-0008) and the engine is Linux only.
  removedOptions = {
    enableSoulver = "Compass's calculator is fend-core; there is no SoulverCore backend.";
    enableNumen = "Compass's calculator is fend-core; there is no Numen backend.";
    enableFirefoxIntegration = "Compass has no browser native messaging host (ADR-0008).";
    enableChromeIntegration = "Compass has no browser native messaging host (ADR-0008).";
    settingOverrides = "Compass reads a single compass.json; use programs.compass.settings.";
    launchd = "Compass runs on Linux only.";
  };
in {
  disabledModules = ["programs/vicinae"];

  imports = lib.flatten [
    (map (x: lib.mkRenamedOptionModule ["programs" "vicinae" x] ["programs" "compass" x]) renamedOptions)
    (map (x: lib.mkRenamedOptionModule ["services" "vicinae" x] ["programs" "compass" x]) renamedOptions)
    (map (x: lib.mkRenamedOptionModule ["programs" "vicinae" "systemd" x] ["programs" "compass" "systemd" x]) renamedSystemdOptions)
    (map (x: lib.mkRenamedOptionModule ["services" "vicinae" "systemd" x] ["programs" "compass" "systemd" x]) renamedSystemdOptions)
    (lib.mapAttrsToList (x: why: lib.mkRemovedOptionModule ["programs" "vicinae" x] why) removedOptions)
  ];

  options.programs.compass = {
    enable = lib.mkEnableOption "the Compass launcher";

    package = lib.mkOption {
      type = lib.types.package;
      default = self.packages.${pkgs.stdenv.hostPlatform.system}.compass;
      defaultText = lib.literalExpression "compass.packages.\${system}.compass";
      description = "The Compass package to install.";
    };

    systemd = {
      enable = lib.mkEnableOption "the compass.service systemd user unit";

      autoStart = lib.mkOption {
        type = lib.types.bool;
        default = true;
        description = "Whether the Compass engine starts with the session target.";
      };

      environment = lib.mkOption {
        type = envVarType;
        default = {};
        description = "Environment variables for the Compass engine.";
        example = lib.literalExpression ''
          {
            COMPASS_LAYER_SHELL = 0;
          }
        '';
      };

      target = lib.mkOption {
        type = lib.types.str;
        default = "graphical-session.target";
        example = "sway-session.target";
        description = "The systemd target that starts and stops compass.service.";
      };
    };

    extensions = lib.mkOption {
      type = lib.types.listOf lib.types.package;
      default = [];
      description = ''
        Extensions to install into `~/.local/share/compass/extensions`.
        The flake's `mkVicinaeExtension` and `mkRayCastExtension` build them.
      '';
    };

    themes = lib.mkOption {
      inherit (tomlFormat) type;
      default = {};
      description = ''
        Themes written to `~/.local/share/compass/themes/<name>.toml`, where Set Theme offers
        them beside the built-in ones. The attribute name is the file name.
      '';
      example = lib.literalExpression ''
        {
          catppuccin-mocha = {
            meta = {
              name = "Catppuccin Mocha";
              description = "Cozy feeling with color-rich accents";
              variant = "dark";
              inherits = "vicinae-dark";
            };
            colors.core = {
              background = "#1E1E2E";
              foreground = "#CDD6F4";
              secondary_background = "#181825";
              border = "#313244";
              accent = "#89B4FA";
            };
          };
        }
      '';
    };

    settings = lib.mkOption {
      inherit (jsonFormat) type;
      default = {};
      description = ''
        Configuration written as JSON to `~/.config/compass/compass.json`. When it is set, the
        file is managed by Home Manager: a change made in the settings view replaces the link
        and is overwritten at the next activation. `compass config schema` prints the schema.
      '';
      example = lib.literalExpression ''
        {
          close_on_focus_loss = true;
          pop_to_root_on_close = true;
          font.normal.size = 12;
        }
      '';
    };
  };

  config = lib.mkIf cfg.enable {
    home.packages = [cfg.package];

    xdg.configFile."compass/compass.json" = lib.mkIf (cfg.settings != {}) {
      source = jsonFormat.generate "compass.json" cfg.settings;
      force = true;
    };

    xdg.dataFile =
      builtins.listToAttrs (
        map (item: {
          name = "compass/extensions/${item.name}";
          value.source = item;
        })
        cfg.extensions
      )
      // lib.mapAttrs' (
        name: theme:
          lib.nameValuePair "compass/themes/${name}.toml" {
            source = tomlFormat.generate "compass-${name}-theme" theme;
          }
      )
      cfg.themes;

    # packaging/systemd/compass.service, with the store path and the target.
    systemd.user.services.compass = lib.mkIf cfg.systemd.enable {
      Unit = {
        Description = "Compass launcher";
        Documentation = ["https://github.com/tuna-os/compass"];
        After = [cfg.systemd.target];
        Requires = ["dbus.socket"];
        PartOf = [cfg.systemd.target];
      };
      Service = {
        Environment =
          lib.mapAttrsToList (key: val: "${key}=${envValueToString val}")
          cfg.systemd.environment;
        Type = "simple";
        ExecStart = "${lib.getExe cfg.package} start --hidden";
        Restart = "on-failure";
        RestartSec = 5;
      };
      Install = lib.mkIf cfg.systemd.autoStart {
        WantedBy = [cfg.systemd.target];
      };
    };
  };
}
