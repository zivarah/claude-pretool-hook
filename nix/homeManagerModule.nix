{
  config,
  lib,
  defaultPackage,
  ...
}:
let
  cfg = config.programs.claude-pretool-hook;
in
{
  options.programs.claude-pretool-hook = {
    enable = lib.mkEnableOption "Claude Code PreToolUse hook";

    package = lib.mkOption {
      type = lib.types.package;
      default = defaultPackage;
      description = "The claude-pretool-hook package to use.";
    };
  };

  config = lib.mkIf cfg.enable { home.packages = [ cfg.package ]; };
}
