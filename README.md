# Claude Code PreToolUse Hook

A configurable [PreToolUse hook][1] for Claude Code that supports complex rules
for allowing, denying, or prompting on tool calls. It includes special handling
for Bash commands with flag and argument-based rules, as well as path-based
rules for file access tools. The hook is designed to be flexible and extensible,
allowing users to define precise policies for tool usage in their Claude Code
agents.

[1]: https://docs.anthropic.com/en/docs/claude-code/hooks

## How it works

Claude Code calls this hook before every tool invocation. The hook binary reads
the tool name and input JSON from stdin, evaluates it against a rules JSON file
(passed via `--rules <path>`), and returns one of three decisions:

- **allow** -- tool call proceeds without prompting
- **deny** -- tool call is blocked with an explanation
- **ask** -- user is prompted to approve or reject

### Tool categories

Tools are divided into two groups based on how they're evaluated:

- **Path-conditional tools** (`Read`, `Write`, `Edit`, `NotebookEdit`, `Glob`,
  `Grep`) -- evaluated against a configurable `DecisionNode` that can be a
  static decision or a conditional (`if/then/else`) checked against file-access
  glob patterns. For example, `Read` might use `ifReadable` (allow if readable,
  else deny) and `Write` might use `ifWritable` (allow if writable, ask if
  readable, else deny). `Glob` and `Grep` resolve their `path` parameter (or
  cwd if absent) against the same file-access patterns.

- **Static tools** (everything else: `LSP`, `Agent`, `Skill`, task tools,
  `ToolSearch`, `WebFetch`, etc.) -- assigned a fixed decision (usually
  "allow"). Any tool name not explicitly listed in the rules defaults to the
  flattened `other` catch-all in `ToolEntry`, which accepts a
  `StaticDecisionNode`.

- **`Bash`** -- special handling. The command string is parsed using
  tree-sitter-bash and evaluated against command decision trees. Shell
  redirects (`>`, `>>`, etc.) automatically get writable path checks.

### Command decision trees

Each Bash command is a decision tree with decisions at every level. The
strictest decision wins: **deny > ask > allow**.

### Core principles

1. **Every entry has a decision.** Commands, subcmds, flags, options,
   values -- all require an explicit `decision` field (or a bare decision
   string).

2. **Unlisted = ask.** If a dict exists for a category and an entry isn't
   found (and no `*` wildcard), the result is "ask".

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

The same logic applies to `subcmds`, `options`, and `positional` dicts:
absent = transparent, present with `*` = wildcard default, present without `*` =
"ask" for unlisted entries.

## Source files

- `main.rs` -- CLI entry point. Reads `--rules <file>` and stdin JSON,
  dispatches by tool name, resolves path-conditional decisions.
- `bash.rs` -- Bash command parsing using tree-sitter-bash.
- `evaluate.rs` -- Core decision tree evaluation logic.
- `decision.rs` -- Decision types, conditions, and merging (deny > ask >
  allow).
- `path.rs` -- Path normalization, glob pattern matching, and conditional
  decision resolution.
- `rules.rs` -- Data structures for deserializing the rules JSON, including
  `WildcardMap` for separating `*` entries from named entries at
  deserialization time.
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
  }
}
```

### Command node

Each command in `tools.Bash.commands` is either a bare decision string or a
full node:

```json
{
  "decision": "ask",
  "subcmds": { "<name>": "<command-node>", "*": "<command-node>" },
  "flags": { "<name>": "<flag-entry>", "*": "<flag-entry>" },
  "options": { "<name>": "<option-entry>", "*": "<option-entry>" },
  "positional": { "<count>": "<positional-def>", "*": "<positional-def>" },
  "isWrapper": false,
  "skipPositional": 0,
  "allowExpansions": false
}
```

All fields except `decision` are optional.

### Flag entry

Either a bare decision string or an object. A flag has exactly one of
`decision` or `positional`:

```json
// Static decision
"-r": "deny"
"-r": { "decision": "deny", "force": false, "aliases": ["--recursive"] }

// Positional overlay -- adds path checks to positional args when this flag
// is present. The flag itself has no standalone decision.
"-i": { "positional": { "*": { "if": "writable", "then": "allow", "else": "ask" } } }
```

### Option entry

Either a bare decision string, a bare conditional, or a full object:

```json
"-m": { "decision": "allow", "allowExpansions": true }
"-f": { "decision": "allow", "values": { "dev": "allow", "prod": "deny", "*": "ask" } }
"-C": { "if": "writable", "then": "allow", "else": "deny" }
```

Both space-separated (`--output /tmp/file`) and equals-separated
(`--output=/tmp/file`) forms are recognized. For the equals form, the
argument is split on the first `=` and the option name is looked up
normally (including alias matching).

### Positional def

Keyed by argument count (`"1"`, `"2"`, etc.) or `"*"` for any count. Values
are either a single decision node (applied uniformly) or an array (one rule
per position):

```json
"positional": {
  "*": "ask",
  "2": [
    { "if": "readable", "then": "allow", "else": "deny" },
    { "if": "writable", "then": "allow", "else": "ask" }
  ]
}
```

When a `"*"` wildcard matches and the def is a single entry (not an array),
the rule applies to **all** positional args.

### Decision node (conditional)

A decision can be a static string or a nested conditional:

```json
"allow"
{ "if": "readable", "then": "allow", "else": "deny" }
{ "if": "writable", "then": "allow", "else": { "if": "readable", "then": "ask", "else": "deny" } }
```

### File access

Glob patterns evaluated in order, last match wins. `!` prefix negates.
`CLAUDE_PROJECT_DIR` is automatically appended to write patterns at runtime.

When `requireReadable` is true, a path must pass the read patterns before
write patterns are checked.
