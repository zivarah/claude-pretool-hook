# Submodule body for `programs.claude-pretool-hook.rules`. Mirrors the JSON
# rule schema deserialized by src/rules.rs — option names and values map
# directly to the JSON the hook reads via `--rules`. Use as:
#
#     rules = lib.mkOption {
#       type = lib.types.submodule (import ./rule-schema.nix { inherit lib; });
#       default = { };
#     };
{ lib }:
let
  decisionType = lib.types.enum [
    "allow"
    "deny"
    "ask"
  ];

  # Recursive type: either a plain decision string or a conditional
  # { if = "readable"|"writable"; then = <conditional>; else = <conditional>; }
  conditionalDecisionType =
    let
      check =
        v:
        (
          builtins.isString v
          && builtins.elem v [
            "allow"
            "deny"
            "ask"
          ]
        )
        || (
          builtins.isAttrs v
          && v ? "if"
          && v ? "then"
          && v ? "else"
          && builtins.elem v."if" [
            "readable"
            "writable"
          ]
          && check v."then"
          && check v."else"
        );
    in
    lib.mkOptionType {
      name = "conditionalDecision";
      description = ''decision string ("allow"|"deny"|"ask") or conditional ({if, then, else})'';
      inherit check;
      merge = lib.options.mergeEqualOption;
    };

  forceOption = lib.mkOption {
    type = lib.types.bool;
    default = false;
    description = "When true, this decision takes priority over non-forced decisions. Conflicting forced decisions result in ask.";
  };

  # Each entry type accepts either a bare decision string ("allow") or the
  # full submodule form. The Rust deserializer handles both.
  valueEntryType = lib.types.either conditionalDecisionType (
    lib.types.submodule {
      options = {
        decision = lib.mkOption {
          type = conditionalDecisionType;
          description = "Decision for this option value.";
        };
        force = forceOption;
        isPattern = lib.mkOption {
          type = lib.types.bool;
          default = false;
          description = ''
            When true, this entry's *key* in the enclosing values dict is
            evaluated as a regex matched against the value rather than an
            exact-match string..
          '';
        };
      };
    }
  );

  # A flag entry is one of:
  #   - A bare decision string ("allow") — shorthand for decision variant.
  #   - { decision, force?, aliases? } — flag with a standalone decision.
  #   - { positional, force?, aliases? } — flag that overlays additional
  #     positional path-access rules when present (e.g., sed's -i).
  # Both decision and positional are optional in the submodule so that
  # Nix's type system can accept either form without needing `oneOf`
  # (which doesn't reliably distinguish submodule variants).
  flagEntryType = lib.types.either decisionType (
    lib.types.submodule {
      options = {
        force = forceOption;
        aliases = lib.mkOption {
          type = lib.types.listOf lib.types.str;
          default = [ ];
          description = "Alternative names for this flag.";
        };
        decision = lib.mkOption {
          type = lib.types.nullOr decisionType;
          default = null;
          description = "Decision for this flag (mutually exclusive with positional).";
        };
        positional = lib.mkOption {
          type = lib.types.nullOr (lib.types.attrsOf positionalDefType);
          default = null;
          description = ''
            Positional rules overlaid when this flag is present.
            Keyed by count ("1", "2", "*"). Merged with base
            positional rules via strictness (deny > ask > allow).
            Mutually exclusive with decision.
          '';
        };
      };
    }
  );

  # `checkFile` body for an option (and, in a later commit, positional)
  # entry: a `values`-shaped dict of decisions plus an `onUnreadable`
  # fallback for when the file can't be read.
  fileCheckType = lib.types.submodule {
    options = {
      values = lib.mkOption {
        type = lib.types.attrsOf valueEntryType;
        default = { };
        description = ''
          Exact / `isPattern: true` / wildcard entries matched against the
          file's contents (rather than the literal value string).
        '';
      };
      onUnreadable = lib.mkOption {
        type = decisionType;
        default = "deny";
        description = ''
          Decision when the referenced file cannot be read: blocked by
          read globs, missing on disk, oversized, or generic I/O error.
        '';
      };
    };
  };

  optionEntryType = lib.types.either conditionalDecisionType (
    lib.types.submodule {
      options = {
        decision = lib.mkOption {
          type = decisionType;
          description = "Decision for this option.";
        };
        force = forceOption;
        aliases = lib.mkOption {
          type = lib.types.listOf lib.types.str;
          default = [ ];
          description = "Alternative names for this option.";
        };
        allowExpansions = lib.mkOption {
          type = lib.types.bool;
          default = false;
          description = "Whether variable/command expansions are allowed in this option's value.";
        };
        values = lib.mkOption {
          type = lib.types.nullOr (lib.types.attrsOf valueEntryType);
          default = null;
          description = "Per-value decision overrides.";
        };
        checkFile = lib.mkOption {
          type = lib.types.nullOr fileCheckType;
          default = null;
          description = ''
            Inspect the contents of the file the value refers to. The
            file's contents are matched against `checkFile.values` using
            the same exact / `isPattern: true` / wildcard rules as a
            normal value lookup.
          '';
        };
      };
    }
  );

  # A positional entry is either:
  #   - the historical bare-decision shape (string, conditional, or
  #     wrapped {decision: ...}), or
  #   - a richer submodule with optional `values` / `checkFile` overlays
  #     matched against the positional arg (and its file contents).
  positionalEntryType = lib.types.either conditionalDecisionType (
    lib.types.submodule {
      options = {
        decision = lib.mkOption {
          type = conditionalDecisionType;
          description = "Base decision for this positional entry.";
        };
        values = lib.mkOption {
          type = lib.types.nullOr (lib.types.attrsOf valueEntryType);
          default = null;
          description = ''
            Per-value decision overrides matched against the literal
            positional arg, using the same exact / `isPattern: true` /
            wildcard rules as option values.
          '';
        };
        checkFile = lib.mkOption {
          type = lib.types.nullOr fileCheckType;
          default = null;
          description = ''
            Inspect the file referenced by this positional arg; the
            file's contents are matched against `checkFile.values`.
          '';
        };
      };
    }
  );

  positionalDefType = lib.types.either positionalEntryType (lib.types.listOf positionalEntryType);

  # Recursive: subcmds references commandNodeType
  commandNodeType = lib.types.either decisionType (
    lib.types.submodule {
      options = {
        decision = lib.mkOption {
          type = lib.types.nullOr decisionType;
          default = null;
          description = "Decision for this command.";
        };
        subcmds = lib.mkOption {
          type = lib.types.nullOr (lib.types.attrsOf (lib.types.either decisionType commandNodeType));
          default = null;
          description = "Subcommand decision trees.";
        };
        flags = lib.mkOption {
          type = lib.types.nullOr (lib.types.attrsOf flagEntryType);
          default = null;
          description = "Flag rules (boolean flags without values).";
        };
        # Named "options" in the Rust struct — flags that take values.
        options = lib.mkOption {
          type = lib.types.nullOr (lib.types.attrsOf optionEntryType);
          default = null;
          description = "Option rules (flags that take values).";
        };
        positional = lib.mkOption {
          type = lib.types.nullOr (lib.types.attrsOf positionalDefType);
          default = null;
          description = ''Positional argument rules keyed by position ("1", "2", "*").'';
        };
        cwdCheck = lib.mkOption {
          type = lib.types.nullOr conditionalDecisionType;
          default = null;
          description = "Conditional decision applied to the working directory when this command runs.";
        };
        isWrapper = lib.mkOption {
          type = lib.types.bool;
          default = false;
          description = "Whether this command wraps another command (e.g. timeout, xargs).";
        };
        skipPositional = lib.mkOption {
          type = lib.types.ints.unsigned;
          default = 0;
          description = "Number of positional args to skip before evaluating inner command (for wrappers).";
        };
        allowExpansions = lib.mkOption {
          type = lib.types.bool;
          default = false;
          description = "Whether variable/command expansions are allowed in arguments.";
        };
      };
    }
  );
in
{
  options = {
    tools = lib.mkOption {
      default = { };
      description = ''
        Per-tool rules. Each value is either a conditional decision
        (plain string or if/then/else) for simple tools, or a
        tool-specific attrset (e.g. Bash with a "commands" key).
      '';
      type = lib.types.attrsOf (
        lib.types.either conditionalDecisionType (
          lib.types.submodule {
            options.commands = lib.mkOption {
              default = { };
              description = "Command decision trees (Bash tool).";
              type = lib.types.attrsOf commandNodeType;
            };
            options.globallyAllowedFlags = lib.mkOption {
              default = [ ];
              description = ''
                Flags that auto-allow a command when they are the sole
                argument (e.g., ["--help" "--version"]). When a command
                is invoked with exactly one of these flags and nothing
                else, the command is allowed regardless of other rules.
              '';
              type = lib.types.listOf lib.types.str;
            };
          }
        )
      );
    };

    fileAccess = {
      read.globPatterns = lib.mkOption {
        default = [ ];
        description = ''
          Glob patterns controlling read access. Evaluated in order,
          last match wins. Prefix with ! to negate. Supports * and **.
        '';
        type = lib.types.listOf lib.types.str;
      };
      write = {
        globPatterns = lib.mkOption {
          default = [ ];
          description = ''
            Glob patterns controlling write access. Evaluated in order,
            last match wins. Prefix with ! to negate. Supports * and **.
          '';
          type = lib.types.listOf lib.types.str;
        };
        requireReadable = lib.mkOption {
          default = false;
          description = ''
            If true, a path must also pass the read glob patterns to be
            considered writable. This avoids duplicating deny patterns
            across read and write rules.
          '';
          type = lib.types.bool;
        };
      };
    };
  };
}
