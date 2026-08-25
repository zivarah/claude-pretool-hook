# Claude Code PreToolUse Hook

A configurable [PreToolUse hook][1] for Claude Code that supports complex rules
for allowing, denying, or prompting on tool calls. It includes special handling
for Bash commands with flag and argument-based rules, as well as path-based
rules for file access tools. The hook is designed to be flexible and extensible,
allowing users to define precise policies for tool usage in their Claude Code
agents.

[1]: https://docs.anthropic.com/en/docs/claude-code/hooks

## How it works

Claude Code calls this hook before every tool invocation. Run the hook with
`--mode claude` or `--mode codex`. The hook binary reads the tool name and input
JSON from stdin, evaluates it against a rules JSON file (passed via
`--rules <path>`), and returns one of three decisions:

- **allow** -- tool call proceeds without prompting
- **deny** -- tool call is blocked with an explanation
- **ask** -- user is prompted to approve or reject

The hook can also abstain, returning no decision so the caller's own approval
flow applies. See [`deferAskInAutoMode`](#deferaskinautomode) and
[Using with OpenAI Codex](#using-with-openai-codex).

### Tool categories

Tools are divided into two groups based on how they're evaluated:

- **Path-conditional tools** (`Read`, `Write`, `Edit`, `NotebookEdit`, `Glob`,
  `Grep`) -- evaluated against a configurable `DecisionNode` that can be a
  static decision or a conditional (`if/then/else`) checked against file-access
  glob patterns. For example, `Read` might use `ifReadable` (allow if readable,
  else deny) and `Write` might use `ifWritable` (allow if writable, ask if
  readable, else deny). `Glob` and `Grep` resolve their `path` parameter (or cwd
  if absent) against the same file-access patterns.

- **Static tools** (everything else: `LSP`, `Agent`, `Skill`, task tools,
  `ToolSearch`, `WebFetch`, etc.) -- assigned a fixed decision (usually
  "allow"). Any tool name not explicitly listed in the rules defaults to the
  flattened `other` catch-all in `ToolEntry`, which accepts a
  `StaticDecisionNode`.

- **`Bash`** -- special handling. The command string is parsed using
  tree-sitter-bash and evaluated against command decision trees.
  - Shell redirects (`>`, `>>`, etc.) automatically get a writable conditional
    path check (writable -> allow, else readable -> ask, else deny).
  - Commands inside command substitutions (e.g. `$(...)`) are recursively
    extracted and evaluated independently. The `globallyAllowedFlags` list (e.g.
    `["--help", "--version"]`) auto-allows any command invoked with exactly one
    of those flags as its sole argument, regardless of other rules. The
    auto-allow also fires when wrapper stripping leaves just the flag (e.g.
    `timeout --help`).

### Command decision trees

Each Bash command is a decision tree with decisions at every level. The
strictest decision wins: **deny > ask > allow**, except when a `force` decision
is in play (see [Forced decisions](#forced-decisions)).

### Core principles

1. **Every entry has a decision.** Commands, subcmds, flags, options, values --
   all require an explicit `decision` field (or a bare decision string).

2. **Unlisted = ask.** If a dict exists for a category and an entry isn't found
   (and no `*` wildcard), the result is "ask".

3. **Absent dict = transparent.** If a category dict doesn't exist on a node,
   that category isn't checked. This is how `{ "decision": "allow" }` allows a
   command with any flags.

4. **`--help`/`--version`**: any command with `--help` or `--version` as its
   sole argument is auto-allowed.

### Dict presence, `*`, and omission

Whether a category dict (subcmds, flags, options, positional) is present on a
node changes the evaluation behavior significantly. The `*` wildcard controls
what happens when an entry isn't found in a present dict. Here's an annotated
example showing all three cases:

```json
// Case 1: No flags dict -- flags are NOT CHECKED.
// `ls`, `ls -la`, `ls --color` all evaluate to "allow"
// because no flags dict means the category is transparent.
"ls": "allow"

// Case 2: Flags dict WITH "*" -- unknown flags use the wildcard.
// `rm file`       -> "ask" (node decision, no flags present)
// `rm -r file`    -> "deny" (-r is listed)
// `rm -v file`    -> "allow" (-v is not listed, uses "*")
"rm": {
  "decision": "ask",
  "flags": {
    "*": "allow",
    "-r": "deny",
    "-rf": "deny"
  }
}

// Case 3: Flags dict WITHOUT "*" -- unknown flags produce "ask".
// `bash`           -> "ask" (node decision, no flags present)
// `bash -c 'code'` -> "deny" (-c is listed)
// `bash -x script` -> "ask" (-x is not listed, no "*")
"bash": {
  "decision": "ask",
  "flags": {
    "-c": "deny"
  }
}
```

The same logic applies to `subcmds`, `options`, and `positional` dicts: absent =
transparent, present with `*` = wildcard default, present without `*` = "ask"
for unlisted entries.

### Forced decisions

Flags, options, option values, and wrapper flags can set `"force": true` to
override the normal "strictest wins" merging. A forced decision takes priority
over any non-forced decision in the same evaluation. Multiple forced decisions
that agree are applied; multiple forced decisions that disagree resolve to "ask"
(treated as a conflict).

The canonical use is `command -v <name>`: even if `<name>` would otherwise be
denied, `command -v` is a read-only lookup that should be allowed. Marking `-v`
as `{"decision": "allow", "force": true}` on the `command` wrapper makes the
inner command auto-allowed (see [Wrappers](#wrappers)).

```json
"command": {
  "decision": "ask",
  "isWrapper": true,
  "flags": {
    "-v": { "decision": "allow", "force": true },
    "-V": { "decision": "allow", "force": true }
  }
}
```

### Wrappers

A command marked `"isWrapper": true` is stripped from the front of the argument
list before the inner command is evaluated. Stripping consumes:

1. The wrapper's own command name.
2. Any of the wrapper's known flags or `option <value>` / `option=value` pairs
   found at the front of the args, including a literal `--` separator (which
   stops flag consumption).
3. `skipPositional` positional arguments after the flags (e.g., `timeout 5 ls`
   declares `skipPositional: 1` to consume the duration).

The remaining args are then evaluated as if they were the original command.
Wrappers chain: `timeout 5 timeout 10 ls` strips both `timeout` invocations.

If a wrapper flag is matched and that flag has `force: true`, wrapper stripping
short-circuits and the entire command is force-allowed (used by
`command -v`/`command -V`).

### Expansion handling

Tree-sitter classifies each argument as either fully literal or containing
non-literal parts (variable expansions like `$VAR`, command substitutions like
`$(...)`, etc.). By default, any non-literal arg in a command produces an "ask"
judgment, since the hook can't statically reason about what the expansion will
resolve to.

Two opt-ins relax this:

- **Node-level `"allowExpansions": true`** -- declared on a command node, every
  arg may contain expansions.

- **Option-level `"allowExpansions": true`** -- declared on a single option
  entry, only the value position of that option may contain an expansion. This
  would allow, for example, `git commit -m "$(...)"`; as long as the inner
  command is deemed safe, this is safe to allow as the resulting string is just
  used as the commit message (never executed).

If a command has any uncovered non-literal arg, an "ask" judgment is added to
the merge (it does not short-circuit, so a more lenient or stricter decision
elsewhere can still win).

### `cwdCheck`

A command node may declare a `cwdCheck` containing a conditional decision that
is resolved against the hook's working directory (passed in via the hook input,
normalized in the same way as positional path checks):

```json
"unzip": {
  "decision": "ask",
  "cwdCheck": {
    "if": "writable",
    "then": "allow",
    "else": { "if": "readable", "then": "ask", "else": "deny" }
  },
  "positional": {
    "*": { "if": "readable", "then": "allow", "else": "deny" }
  }
}
```

This is intended for commands that write into the cwd implicitly --
`unzip archive.zip` extracts under the cwd whether or not `-d` is given, so the
cwd should pass the writable check even if no flag points at it.

## Source files

- `main.rs` -- CLI entry point. Reads `--mode <claude|codex>`, `--rules <file>`,
  and stdin JSON. It dispatches by tool name and resolves path-conditional
  decisions.
- `bash.rs` -- Bash command parsing using tree-sitter-bash.
- `evaluate.rs` -- Core decision tree evaluation logic.
- `decision.rs` -- Decision types, conditions, and merging (deny > ask > allow).
- `path.rs` -- Path normalization, glob pattern matching, and conditional
  decision resolution.
- `rules.rs` -- Data structures for deserializing the rules JSON, including
  `WildcardMap` for separating `*` entries from named entries at deserialization
  time.
- `input.rs` -- Input types for each tool's JSON payload.

## Rules JSON schema

The rules JSON has this top-level structure:

```json
{
  "tools": {
    "Bash": { "commands": { ... } },
    "Read": "<decision-or-conditional>",
    "Write": "<decision-or-conditional>",
    "Edit": "<decision-or-conditional>",
    "NotebookEdit": "<decision-or-conditional>",
    "Glob": "<decision-or-conditional>",
    "Grep": "<decision-or-conditional>",
    ...
  },
  "fileAccess": {
    "read": { "globPatterns": ["**", "!**/*.secret*"] },
    "write": { "globPatterns": ["/tmp/**"], "requireReadable": true }
  },
  "deferAskInAutoMode": false
}
```

### `deferAskInAutoMode`

In Claude Code's **auto** permission mode, Claude decides for itself any tool
call the permission rules leave unresolved. An "ask" from this hook
short-circuits that and prompts you instead, which makes auto mode no less
interactive than the default mode.

Setting `"deferAskInAutoMode": true` (default `false`) will cause the hook to
abstain from making a decision at all when running in claude's auto mode.
"allow" and "deny" are still emitted. Every other permission mode is unaffected.

The deferral covers implicit asks too (an unlisted command, an unmatched flag),
not just rules that spell out `"ask"`, which is why it is opt-in. A payload the
hook cannot parse still asks, since the permission mode is unknown at that
point.

### Command node

Each command in `tools.Bash.commands` is either a bare decision string or a full
node:

```json
{
  "decision": "ask",
  "subcmds": { "<name>": "<command-node>", "*": "<command-node>" },
  "flags": { "<name>": "<flag-entry>", "*": "<flag-entry>" },
  "options": { "<name>": "<option-entry>", "*": "<option-entry>" },
  "positional": { "<count>": "<positional-def>", "*": "<positional-def>" },
  "cwdCheck": "<decision-or-conditional>",
  "isWrapper": false,
  "skipPositional": 0,
  "allowExpansions": false
}
```

All fields except `decision` are optional.

### Flag entry

Either a bare decision string or an object. A flag has exactly one of `decision`
or `positional`:

```json
// Static decision
"-r": "deny"
"-r": { "decision": "deny", "aliases": ["--recursive"] }

// Positional overlay -- adds path checks to positional args when this flag
// is present. The flag itself has no standalone decision. By default the
// overlay's judgments merge with the command node's top-level `positional`
// (strictest wins); pass `overridePositional: true` to suppress the parent
// instead. See [Flag and option positional
// overlays](#flag-and-option-positional-overlays).
"-i": { "positional": { "*": { "if": "writable", "then": "allow", "else": "ask" } } }
"-r": {
  "positional": { "*": { "if": "readable", "then": "allow", "else": "deny" } },
  "overridePositional": true
}
```

### Option entry

Either a bare decision string, a bare conditional, or a full object:

```json
"-m": { "decision": "allow", "allowExpansions": true }
"-f": { "decision": "allow", "values": { "dev": "allow", "prod": "deny", "*": "ask" } }
"-C": { "if": "writable", "then": "allow", "else": "deny" }
```

A bare conditional is shorthand for
`{"decision": "ask", "values": {"*": <conditional>}}` -- i.e., the option's
_value_ is path-checked by the conditional. The `-C` example above evaluates the
value of `git -C <dir>` against the writable patterns.

Full-object fields:

- `decision` (required when not bare) -- the standalone decision when no
  `values` dict matches.
- `aliases` -- additional names that resolve to this entry (e.g., `-X` aliased
  to `--request`).
- `force` -- see [Forced decisions](#forced-decisions). On individual `values`
  entries, `force` is also accepted via the `DecisionSpec` shape:
  `{"decision": ..., "force": true}`.
- `allowExpansions` -- see [Expansion handling](#expansion-handling).
- `values` -- per-value rules (decision string, conditional, or full spec with
  `force` / `isPattern`). See
  [Value lookup and isPattern](#value-lookup-and-ispattern). Wildcard `*` falls
  through to "ask" when absent.
- `checkFile` -- treat the value as a path and run a second `values`-shaped
  lookup against the contents of that file. See [`checkFile`](#checkfile).
- `positional` -- positional rules applied to args appearing _after_ this
  option's value consumes its argument. Same shape as the command node's
  `positional` (keyed by count). See
  [Flag and option positional overlays](#flag-and-option-positional-overlays).
- `overridePositional` -- when `true` and this option is present, the command
  node's top-level `positional` rule is skipped. Other matched flags'/options'
  positional overlays still apply. Default `false`. See
  [Flag and option positional overlays](#flag-and-option-positional-overlays).

Both space-separated (`--output /tmp/file`) and equals-separated
(`--output=/tmp/file`) forms are recognized. For the equals form, the argument
is split on the first `=` and the option name is looked up normally (including
alias matching).

### Positional def

Keyed by argument count (`"1"`, `"2"`, etc.) or `"*"` for any count. Values are
either a single entry (applied uniformly) or an array (one rule per position):

```json
"positional": {
  "*": "ask",
  "2": [
    { "if": "readable", "then": "allow", "else": "deny" },
    { "if": "writable", "then": "allow", "else": "ask" }
  ]
}
```

When a `"*"` wildcard matches and the def is a single entry (not an array), the
rule applies to **all** positional args.

Each entry can also use the richer object form to attach `values` and/or
`checkFile` overlays to the positional, mirroring an option entry:

```json
"positional": {
  "*": {
    "decision": "allow",
    "values": {
      "*": "allow",
      "\\bsystem\\(": { "decision": "ask", "isPattern": true }
    }
  }
}
```

The base `decision` is classified as before. When `values` is present, the
positional arg's literal text is run through the same lookup as
[Value lookup and isPattern](#value-lookup-and-ispattern). When `checkFile` is
present, the positional is treated as a path and the file contents are scanned
the same way -- useful for `awk 'script' file` where the first positional is the
script.

### Flag and option positional overlays

In addition to the command node's `positional` rule, **flag** and **option**
entries can each declare their own `positional` overlay -- extra positional
rules that apply only when that flag or option is present in the invocation.
This is how `sed -i FILE` overlays a writable check on top of `sed`'s base
positional rule, or how `sed -e EXPR FILE` adds a readability check on the file
arg.

Three rules govern how these overlays interact with the command-level
`positional`:

1. **Each matched flag/option contributes its overlay's judgments to the
   merge.** A command node's top-level `positional` is evaluated normally, and
   every present flag/option with a `positional` field _also_ runs against the
   same positional args. All resulting judgments go through the standard
   "strictest wins" merge (deny > ask > allow), so an overlay can only _tighten_
   the parent's decision, not relax it.

2. **`overridePositional: true` on a flag or option suppresses the command
   node's top-level `positional`.** When such a flag/option is matched, the
   command node's `positional` rule is skipped entirely for the merge. Overlays
   from _other_ matched flags/options still apply -- override only replaces the
   _parent's_ `positional`, not any sibling overlay.

3. **Multiple `overridePositional` flags/options stack additively.** If several
   matched flags/options each set `overridePositional: true`, the parent's
   `positional` is skipped once, and every participating overlay contributes its
   own judgments. They merge with each other (and with non-override overlays)
   under the usual strictest-wins rule.

The override field exists because the parent's `positional` rule sees positional
args by count, not by meaning. For some commands a flag or option changes what
positional args mean: `sed 'SCRIPT' FILE` has the script in position 1, but
`sed -e 'SCRIPT' FILE` makes position 1 the file instead -- and a script-pattern
check shouldn't fire on a file path. Similarly, `grep PATTERN FILE` has a
pattern in position 1, but `grep -r DIR` makes position 1 a directory. The
flag/option declares "when I'm present, ignore the parent's positional
assumptions and use mine."

```jsonc
// sed: parent positional treats arg 1 as a script and screens it for
// dangerous constructs. When -e or -f is present, the lone positional
// is a file (the script came from the option), so the parent's rule
// would mis-screen it -- overridePositional turns it off and the
// option's own positional adds an ifReadable check on the file.
"sed": {
  "decision": "allow",
  "options": {
    "-e": {
      "decision": "allow",
      "values": { /* danger-pattern check against the script */ },
      "overridePositional": true,
      "positional": {
        "*": { "if": "readable", "then": "allow", "else": "deny" }
      }
    }
  },
  "positional": {
    "1": [ { "decision": "allow", "values": { /* script danger check */ } } ],
    "2": [
      { "decision": "allow", "values": { /* script danger check */ } },
      { "if": "readable", "then": "allow", "else": "deny" }
    ]
  }
}
```

Notes:

- `overridePositional: true` with no `positional` field on the flag/option
  simply suppresses the parent's `positional` -- the flag/option's positional
  contribution to the merge is empty. Useful when a flag/option fundamentally
  invalidates any parent-level positional check (rare).
- Both flag shapes (`{ "decision": ... }` and `{ "positional": ... }`) accept
  `overridePositional`. The bare string form (`"-r": "deny"`) does not --
  there's no object on which to set the field. A decision-form flag with
  override but no `positional` simply suppresses the parent for the merge while
  contributing its own decision normally.
- The option's `values` / `checkFile` checks against the option's own _value_
  are independent of `overridePositional`; they always run.

### Value lookup and `isPattern`

A `values` dict resolves a value against three kinds of entry, with results
merged through the normal "strictest wins" path:

1. **Exact match** -- a non-`isPattern` entry whose key equals the value.
2. **Regex match** -- any entry marked `isPattern: true` whose key, compiled as
   a regex (Rust `regex` crate syntax), matches the value. Multiple patterns may
   match; each contributes a judgment.
3. **Wildcard `*`** -- fires only when neither (1) nor (2) matched.

Pattern entries must use the object form so the marker is explicit; bare strings
(`"foo": "allow"`) are always exact-match.

```json
"-e": {
  "decision": "allow",
  "values": {
    "*": "allow",
    "\\bsystem\\(": { "decision": "ask", "isPattern": true }
  }
}
```

A malformed regex emits a deny judgment with the parse error in the reason, so a
broken pattern cannot silently fall through to the wildcard.

`isPattern` is only valid inside value-context dicts (`values` on options and
positionals, and `checkFile.values`). The type system prevents it from appearing
on command-, flag-, or bare-positional decisions.

### `checkFile`

`checkFile` on an option (or rich positional entry) reads the file the value
refers to and matches its **contents** against an inner `values` dict using the
same exact / `isPattern` / `*` rules:

```json
"-f": {
  "decision": "allow",
  "values": {
    "*": { "if": "readable", "then": "allow", "else": "deny" }
  },
  "checkFile": {
    "onUnreadable": "deny",
    "values": {
      "*": "allow",
      "\\bsystem\\(": { "decision": "ask", "isPattern": true }
    }
  }
}
```

The path goes through the existing read-globs gate before the file is opened;
failures (blocked by globs, missing, oversized, not valid UTF-8, generic I/O
error) all resolve to the `onUnreadable` decision (default `deny`). Files are
capped at 1 MiB.

### Decision node (conditional)

A decision can be a static string or a nested conditional:

```json
"allow"
{ "if": "readable", "then": "allow", "else": "deny" }
{ "if": "writable", "then": "allow", "else": { "if": "readable", "then": "ask", "else": "deny" } }
```

### File access

Glob patterns evaluated in order, last match wins. `!` prefix negates. A path
with no matching pattern is treated as not matching (i.e., not readable / not
writable). Paths are normalized lexically (`.` / `..` resolved, relative paths
joined to the hook's cwd) before matching -- no filesystem access is performed.

If the `CLAUDE_PROJECT_DIR` environment variable is set, `<dir>/**` is prepended
to the read/write patterns at startup, so the project root is readable/writable
by default. You can supply explicit negations to override this (e.g.,
`!<dir>/**`).

When `requireReadable` is true on `write`, a path must pass the read patterns
before the write patterns are checked. This means a denylist entry on `read`
(e.g., `!**/*.secret*`) automatically also blocks writes to that path, even if
`glob_patterns` would otherwise allow it.

## Using with OpenAI Codex

The same binary and the same rules file also work as an [OpenAI Codex][2]
`PreToolUse` hook. Codex's native hook contract closely mirrors Claude Code's:
it sends nearly the same stdin schema, so no separate build is needed. Tool
dispatch is keyed on the `tool_name` field in the payload, not on which agent
invoked the hook.

The caller must select the Codex response protocol with `--mode codex` since the
default is to assume Claude.

[2]: https://developers.openai.com/codex/hooks

### Enabling

Codex hooks are off by default and must be opted into with
`[features].codex_hooks = true` in `~/.codex/config.toml`, then registered under
`[[hooks.PreToolUse]]`. With the home-manager module, set
`programs.claude-pretool-hook.configureCodexHook = true` to generate both.
Equivalent manual config:

```toml
[features]
codex_hooks = true

[[hooks.PreToolUse]]
matcher = ".*"

[[hooks.PreToolUse.hooks]]
type = "command"
command = "/path/to/claude-pretool-hook-codex-wrapped"
# The wrapper adds --mode codex and --rules.
```

**Trust step:** on first use -- and again whenever the command's path changes
(e.g. after a Nix rebuild) -- Codex prompts you to review and trust the hook.
When managing via nix, this is somewhat problematic as the approval attempts to
modify the readonly config file managed by nix. You can use the following to
automatically trust the hook:

```nix
let
  codexHookKey = "${config.home.homeDirectory}/.codex/config.toml:pre_tool_use:0:0";
  codexHookIdentity = {
    event_name = "pre_tool_use";
    matcher = ".*";
    hooks = [
      {
        type = "command";
        command = config.programs.claude-pretool-hook.codexWrappedCommand;
        timeout = 600;
        async = false;
      }
    ];
  };
  codexHookHash = "sha256:${builtins.hashString "sha256" (builtins.toJSON codexHookIdentity)}";
in
{
  programs.codex.settings.hooks.state.${codexHookKey}.trusted_hash = codexHookHash;
}
```

But this may be fragile.
