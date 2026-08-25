{
  config,
  lib,
  pkgs,
  defaultPackage,
  ...
}:
let
  cfg = config.programs.claude-pretool-hook;

  # Strip null / [] / false / 0 from the typed rules so the produced
  # JSON contains only meaningful entries. The Rust deserializer
  # annotates nearly every field with #[serde(default)], so the
  # stripped form decodes back to the same values.
  cleanJson =
    val:
    if builtins.isList val then
      map cleanJson val
    else if builtins.isAttrs val then
      lib.filterAttrs (
        _: v:
        v != null
        && !(builtins.isList v && v == [ ])
        && !(builtins.isBool v && v == false)
        && !(builtins.isInt v && v == 0)
      ) (lib.mapAttrs (_: cleanJson) val)
    else
      val;

  cleanedRules = cleanJson cfg.rules;

  # Need to check individual attributes since the defaults for these attrs are
  # empty lists / objects.
  rulesPopulated =
    cfg.rules.tools != { }
    || cfg.rules.fileAccess.read.globPatterns != [ ]
    || cfg.rules.fileAccess.write.globPatterns != [ ]
    || cfg.rules.fileAccess.write.requireReadable;

  generatedRulesFile = pkgs.writeText "claude-pretool-hook-rules.json" (builtins.toJSON cleanedRules);

  claudeWrappedScript = pkgs.writeShellScriptBin "claude-pretool-hook-wrapped" ''
    exec ${lib.getExe cfg.package} --mode claude --rules ${cfg.rulesFile}
  '';

  codexWrappedScript = pkgs.writeShellScriptBin "claude-pretool-hook-codex-wrapped" ''
    exec ${lib.getExe cfg.package} --mode codex --rules ${cfg.rulesFile}
  '';
in
{
  options.programs.claude-pretool-hook = {
    enable = lib.mkEnableOption "Claude Code PreToolUse hook";

    package = lib.mkOption {
      type = lib.types.package;
      default = defaultPackage;
      description = "The claude-pretool-hook package to use.";
    };

    rules = lib.mkOption {
      type = lib.types.submodule (import ./rule-schema.nix { inherit lib; });
      default = { };
      description = ''
        Typed rule tree compiled to a JSON file consumed by the hook
        via `--rules`. See ${./rule-schema.nix} for the schema.
      '';
    };

    configureHook = lib.mkOption {
      type = lib.types.bool;
      default = rulesPopulated;
      defaultText = lib.literalMD "`true` when `rules` has any populated field, `false` otherwise.";
      description = ''
        When true, append a PreToolUse entry to
        `programs.claude-code.settings.hooks.PreToolUse` that runs the wrapper
        script (which runs the hook in Claude mode with `rulesFile`).
        Defaults to true when `rules` has any populated field, false otherwise.
        Set to false if you want to customize how the hook is configured.

        Requires the `programs.claude-code` home-manager module to be
        loaded; otherwise the assignment errors at evaluation time.
      '';
    };

    configureCodexHook = lib.mkOption {
      type = lib.types.bool;
      default = false;
      description = ''
        When true, enable `[features].hooks` and append a PreToolUse entry to
        `programs.codex.settings.hooks.PreToolUse`. This lets the same rules
        govern OpenAI Codex tool calls.
      '';
    };

    rulesFile = lib.mkOption {
      type = lib.types.path;
      default = generatedRulesFile;
      defaultText = lib.literalMD "JSON file generated from `rules`.";
      description = "JSON file to use for rules. Defaults to being generated from `rules`.";
    };

    command = lib.mkOption {
      type = lib.types.str;
      readOnly = true;
      description = "Path to the bare hook binary, equivalent to `lib.getExe cfg.package`.";
    };

    wrappedCommand = lib.mkOption {
      type = lib.types.str;
      readOnly = true;
      description = ''
        Path to a wrapper script that exec's the hook binary with
        `--mode claude` and `--rules` pointing at `rulesFile`. Use this as the
        `command` field of a Claude Code hook entry.
      '';
    };

    codexWrappedCommand = lib.mkOption {
      type = lib.types.str;
      readOnly = true;
      description = ''
        Path to a wrapper script that exec's the hook binary with
        `--mode codex` and `--rules` pointing at `rulesFile`. Use this as the
        `command` field of an OpenAI Codex hook entry.
      '';
    };
  };

  config = lib.mkIf cfg.enable (
    lib.mkMerge [
      {
        programs.claude-pretool-hook = {
          command = lib.getExe cfg.package;
          wrappedCommand = lib.getExe claudeWrappedScript;
          codexWrappedCommand = lib.getExe codexWrappedScript;
        };
      }
      (lib.mkIf cfg.configureHook {
        programs.claude-code.settings.hooks.PreToolUse = [
          {
            matcher = ".*";
            hooks = [
              {
                type = "command";
                command = lib.getExe claudeWrappedScript;
              }
            ];
          }
        ];
      })
      (lib.mkIf cfg.configureCodexHook {
        programs.codex.settings = {
          features.hooks = true;
          hooks.PreToolUse = [
            {
              matcher = ".*";
              hooks = [
                {
                  type = "command";
                  command = lib.getExe codexWrappedScript;
                }
              ];
            }
          ];
        };
      })
    ]
  );
}
