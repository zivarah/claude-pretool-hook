use std::collections::HashMap;

use serde::Deserialize;

use crate::decision::{ConditionalDecisionNode, Decision, DecisionNode, StaticDecisionNode};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Rules {
    #[serde(default)]
    pub tools: ToolEntry,
    pub file_access: FileAccess,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ToolEntry {
    pub bash: Option<BashRules>,
    pub read: Option<DecisionNode>,
    pub notebook_edit: Option<DecisionNode>,
    pub edit: Option<DecisionNode>,
    pub write: Option<DecisionNode>,
    pub glob: Option<DecisionNode>,
    pub grep: Option<DecisionNode>,
    #[serde(default, flatten)]
    pub other: HashMap<String, StaticDecisionNode>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BashRules {
    #[serde(default)]
    pub commands: HashMap<String, CommandNode>,
    /// Flags that auto-allow a command when they are the sole argument
    /// (e.g., `["--help", "--version"]`). When a command is invoked with
    /// exactly one of these flags and nothing else, the command is allowed
    /// regardless of other rules.
    #[serde(default)]
    pub globally_allowed_flags: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct FileAccess {
    #[serde(default)]
    pub read: AccessRules,
    #[serde(default)]
    pub write: WriteRules,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WriteRules {
    #[serde(default)]
    pub glob_patterns: Vec<String>,
    #[serde(default)]
    pub require_readable: bool,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccessRules {
    #[serde(default)]
    pub glob_patterns: Vec<String>,
}

// ---------------------------------------------------------------------------
// WildcardMap — a HashMap with a separate optional wildcard ("*") entry.
// Deserializes from a JSON object, extracting the "*" key if present.
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct WildcardMap<T> {
    pub entries: HashMap<String, T>,
    pub wildcard: Option<Box<T>>,
}

impl<T> WildcardMap<T> {
    /// Returns a reference to the wildcard entry, if present.
    pub fn wildcard(&self) -> Option<&T> {
        self.wildcard.as_deref()
    }
}

impl<T> Default for WildcardMap<T> {
    fn default() -> Self {
        Self {
            entries: HashMap::new(),
            wildcard: None,
        }
    }
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for WildcardMap<T> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let mut entries: HashMap<String, T> = HashMap::deserialize(deserializer)?;
        let wildcard = entries.remove("*").map(Box::new);
        Ok(WildcardMap { entries, wildcard })
    }
}

// ---------------------------------------------------------------------------
// Decision spec: a DecisionNode with an optional force flag.
// Deserializes from any DecisionNode form (bare string, bare conditional,
// or {"decision":...}) plus an optional "force" field.
//
// The Full variant is tried first so that {"decision":...,"force":true}
// captures the force field before the Bare(DecisionNode) variant can swallow
// the whole object (since DecisionNode also accepts {"decision":...} objects).
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct DecisionSpec {
    pub node: DecisionNode,
    pub force: bool,
    pub is_pattern: bool,
}

impl<'de> Deserialize<'de> for DecisionSpec {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            // Object with a "decision" key — tried first to capture the
            // optional "force" / "isPattern" fields before DecisionNode
            // consumes the object.
            Full {
                decision: DecisionNode,
                #[serde(default)]
                force: bool,
                #[serde(default, rename = "isPattern")]
                is_pattern: bool,
            },
            // Bare string ("allow") or bare conditional ({"if":...}).
            Bare(DecisionNode),
        }
        match Raw::deserialize(deserializer)? {
            Raw::Full {
                decision,
                force,
                is_pattern,
            } => Ok(DecisionSpec {
                node: decision,
                force,
                is_pattern,
            }),
            Raw::Bare(node) => Ok(DecisionSpec {
                node,
                force: false,
                is_pattern: false,
            }),
        }
    }
}

// ---------------------------------------------------------------------------
// Command node — the recursive decision tree for bash commands.
// Deserializes from a bare string ("allow") or the full object.
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct CommandNode {
    pub decision: Option<Decision>,
    pub subcmds: Option<WildcardMap<CommandNode>>,
    pub flags: Option<WildcardMap<FlagEntry>>,
    pub options: Option<WildcardMap<OptionEntry>>,
    pub positional: Option<WildcardMap<PositionalDef>>,
    pub cwd_check: Option<DecisionNode>,
    pub is_wrapper: bool,
    pub skip_positional: usize,
    pub allow_expansions: bool,
}

impl<'de> Deserialize<'de> for CommandNode {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Plain(Decision),
            Full(Box<RawFull>),
        }

        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct RawFull {
            decision: Option<Decision>,
            #[serde(default)]
            subcmds: Option<WildcardMap<CommandNode>>,
            #[serde(default)]
            flags: Option<WildcardMap<FlagEntry>>,
            #[serde(default)]
            options: Option<WildcardMap<OptionEntry>>,
            #[serde(default)]
            positional: Option<WildcardMap<PositionalDef>>,
            #[serde(default)]
            cwd_check: Option<DecisionNode>,
            #[serde(default)]
            is_wrapper: bool,
            #[serde(default)]
            skip_positional: usize,
            #[serde(default)]
            allow_expansions: bool,
        }
        match Raw::deserialize(deserializer)? {
            Raw::Plain(decision) => Ok(CommandNode {
                decision: Some(decision),
                subcmds: None,
                flags: None,
                options: None,
                positional: None,
                cwd_check: None,
                is_wrapper: false,
                skip_positional: 0,
                allow_expansions: false,
            }),
            Raw::Full(f) => Ok(CommandNode {
                decision: f.decision,
                subcmds: f.subcmds,
                flags: f.flags,
                options: f.options,
                positional: f.positional,
                cwd_check: f.cwd_check,
                is_wrapper: f.is_wrapper,
                skip_positional: f.skip_positional,
                allow_expansions: f.allow_expansions,
            }),
        }
    }
}

// ---------------------------------------------------------------------------
// Flag entry — a flag rule with optional force and aliases.
// Deserializes from a bare string ("allow") or object.
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct FlagEntry {
    pub kind: FlagKind,
    pub force: bool,
    pub aliases: Vec<String>,
}

#[derive(Debug)]
pub enum FlagKind {
    /// Flag with a standalone decision (e.g., `--verbose` → allow).
    Decision(Decision),
    /// Flag that overlays additional positional path-access rules when present
    /// (e.g., `-i` adds a writable check to file args).
    Positional(WildcardMap<PositionalDef>),
}

impl<'de> Deserialize<'de> for FlagEntry {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Plain(Decision),
            WithDecision {
                decision: Decision,
                #[serde(default)]
                force: bool,
                #[serde(default)]
                aliases: Vec<String>,
            },
            WithPositional {
                positional: WildcardMap<PositionalDef>,
                #[serde(default)]
                force: bool,
                #[serde(default)]
                aliases: Vec<String>,
            },
        }
        match Raw::deserialize(deserializer)? {
            Raw::Plain(decision) => Ok(FlagEntry {
                kind: FlagKind::Decision(decision),
                force: false,
                aliases: vec![],
            }),
            Raw::WithDecision {
                decision,
                force,
                aliases,
            } => Ok(FlagEntry {
                kind: FlagKind::Decision(decision),
                force,
                aliases,
            }),
            Raw::WithPositional {
                positional,
                force,
                aliases,
            } => Ok(FlagEntry {
                kind: FlagKind::Positional(positional),
                force,
                aliases,
            }),
        }
    }
}

// ---------------------------------------------------------------------------
// Option entry — a flag-with-value rule.
// Deserializes from a bare string ("allow") or object.
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct OptionEntry {
    pub decision: Decision,
    pub force: bool,
    pub aliases: Vec<String>,
    pub allow_expansions: bool,
    pub values: Option<WildcardMap<DecisionSpec>>,
    /// When set, the value is treated as a path to a file; the file's
    /// contents are read and matched against `check_file.values` using the
    /// same exact/pattern/wildcard rules as a normal value lookup.
    pub check_file: Option<FileCheck>,
}

/// Opt-in companion to `OptionEntry.values` (and the analogous positional
/// field) that runs a `values`-shaped dict against the *contents* of the
/// referenced file rather than the literal value string.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileCheck {
    /// Exact-match / `isPattern: true` / wildcard entries, matched against
    /// the file's contents.
    pub values: WildcardMap<DecisionSpec>,
    /// Decision when the file cannot be read (path blocked by file-access
    /// globs, missing on disk, oversized, or a generic I/O error). Defaults
    /// to `deny` — if the rule declared an intent to inspect contents,
    /// failing to inspect cannot resolve to allow.
    #[serde(default = "default_on_unreadable")]
    pub on_unreadable: Decision,
}

fn default_on_unreadable() -> Decision {
    Decision::Deny
}

impl<'de> Deserialize<'de> for OptionEntry {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Plain(Decision),
            // A bare conditional like `ifWritable` used as the entire option
            // entry.  Treated as `values: {"*": <conditional>}` — i.e., the
            // option's value is always path-checked by this conditional.
            BareConditional(ConditionalDecisionNode),
            Full {
                decision: Decision,
                #[serde(default)]
                force: bool,
                #[serde(default)]
                aliases: Vec<String>,
                #[serde(default, rename = "allowExpansions")]
                allow_expansions: bool,
                #[serde(default)]
                values: Option<WildcardMap<DecisionSpec>>,
                #[serde(default, rename = "checkFile")]
                check_file: Option<FileCheck>,
            },
        }
        match Raw::deserialize(deserializer)? {
            Raw::Plain(decision) => Ok(OptionEntry {
                decision,
                force: false,
                aliases: vec![],
                allow_expansions: false,
                values: None,
                check_file: None,
            }),
            Raw::BareConditional(cond) => Ok(OptionEntry {
                decision: Decision::Ask,
                force: false,
                aliases: vec![],
                allow_expansions: false,
                values: Some(WildcardMap {
                    entries: HashMap::new(),
                    wildcard: Some(Box::new(DecisionSpec {
                        node: DecisionNode::Conditional(Box::new(cond)),
                        force: false,
                        is_pattern: false,
                    })),
                }),
                check_file: None,
            }),
            Raw::Full {
                decision,
                force,
                aliases,
                allow_expansions,
                values,
                check_file,
            } => Ok(OptionEntry {
                decision,
                force,
                aliases,
                allow_expansions,
                values,
                check_file,
            }),
        }
    }
}

// ---------------------------------------------------------------------------
// Positional rules — keyed by count ("1", "2", "*").
// ---------------------------------------------------------------------------

/// The value is either a single entry or an array of entries.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum PositionalDef {
    Single(DecisionNode),
    Array(Vec<DecisionNode>),
}

// ---------------------------------------------------------------------------
// Lookup helpers
// ---------------------------------------------------------------------------

/// Look up a flag/option name in a map, checking aliases.
/// Only searches named entries, not the wildcard.
pub fn lookup_with_alias<'a, T: HasAliases>(name: &str, map: &'a WildcardMap<T>) -> Option<&'a T> {
    if let Some(entry) = map.entries.get(name) {
        return Some(entry);
    }
    map.entries
        .values()
        .find(|entry| entry.aliases().iter().any(|a| a == name))
}

pub trait HasAliases {
    fn aliases(&self) -> &[String];
}

impl HasAliases for FlagEntry {
    fn aliases(&self) -> &[String] {
        &self.aliases
    }
}

impl HasAliases for OptionEntry {
    fn aliases(&self) -> &[String] {
        &self.aliases
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_flag(decision: Decision, aliases: &[&str]) -> FlagEntry {
        FlagEntry {
            kind: FlagKind::Decision(decision),
            force: false,
            aliases: aliases.iter().map(|s| s.to_string()).collect(),
        }
    }

    fn make_flags() -> WildcardMap<FlagEntry> {
        let mut entries = std::collections::HashMap::new();
        entries.insert(
            "--verbose".into(),
            make_flag(Decision::Allow, &["--debug", "-v"]),
        );
        entries.insert("--force".into(), make_flag(Decision::Deny, &[]));
        WildcardMap {
            entries,
            wildcard: None,
        }
    }

    #[test]
    fn lookup_direct_match() {
        let flags = make_flags();
        let entry = lookup_with_alias("--verbose", &flags);
        assert!(entry.is_some());
        assert!(matches!(
            entry.unwrap().kind,
            FlagKind::Decision(Decision::Allow)
        ));
    }

    #[test]
    fn lookup_alias_match() {
        let flags = make_flags();
        let entry = lookup_with_alias("--debug", &flags);
        assert!(entry.is_some());
        assert!(matches!(
            entry.unwrap().kind,
            FlagKind::Decision(Decision::Allow)
        ));
    }

    #[test]
    fn lookup_short_alias_match() {
        let flags = make_flags();
        let entry = lookup_with_alias("-v", &flags);
        assert!(entry.is_some());
        assert!(matches!(
            entry.unwrap().kind,
            FlagKind::Decision(Decision::Allow)
        ));
    }

    fn test_rules() -> Rules {
        let json = include_str!("../tests/fixtures/test_rules.json");
        serde_json::from_str(json).expect("test rules should parse")
    }

    fn bash_rules(rules: &Rules) -> &BashRules {
        rules.tools.bash.as_ref().expect("Bash rules should exist")
    }

    #[test]
    fn deserialize_full_rules() {
        let rules = test_rules();
        let bash = bash_rules(&rules);
        // Real commands from the fixture
        assert!(bash.commands.contains_key("ls"));
        assert!(bash.commands.contains_key("cargo"));
        assert!(bash.commands.contains_key("nix"));
        assert!(!rules.file_access.read.glob_patterns.is_empty());
        assert!(!rules.file_access.write.glob_patterns.is_empty());
    }

    #[test]
    fn deserialize_command_node_fields() {
        let rules = test_rules();
        let bash = bash_rules(&rules);

        // timeout is a wrapper with skipPositional=1
        let wrapper = &bash.commands["timeout"];
        assert!(wrapper.is_wrapper);
        assert_eq!(wrapper.skip_positional, 1);

        // env has allowExpansions: true
        let env_node = &bash.commands["env"];
        assert!(env_node.allow_expansions);

        // cargo has subcmds including "test", no wildcard
        let cargo = &bash.commands["cargo"];
        assert!(cargo.subcmds.is_some());
        let subcmds = cargo.subcmds.as_ref().unwrap();
        assert!(subcmds.entries.contains_key("test"));
        assert!(subcmds.wildcard().is_none());

        // npm has subcmds with a "*" wildcard
        let npm = &bash.commands["npm"];
        let npm_subcmds = npm.subcmds.as_ref().unwrap();
        assert!(npm_subcmds.wildcard().is_some());
    }

    #[test]
    fn deserialize_flag_entry_with_force() {
        let entry: FlagEntry =
            serde_json::from_str(r#"{"decision":"allow","force":true}"#).unwrap();
        assert!(matches!(entry.kind, FlagKind::Decision(Decision::Allow)));
        assert!(entry.force);
    }

    #[test]
    fn deserialize_decision_spec_with_force() {
        let spec: DecisionSpec =
            serde_json::from_str(r#"{"decision":"deny","force":true}"#).unwrap();
        assert!(matches!(spec.node, DecisionNode::Static(Decision::Deny)));
        assert!(spec.force);
    }

    #[test]
    fn deserialize_decision_spec_bare_string() {
        let spec: DecisionSpec = serde_json::from_str(r#""allow""#).unwrap();
        assert!(matches!(spec.node, DecisionNode::Static(Decision::Allow)));
        assert!(!spec.force);
    }

    #[test]
    fn deserialize_decision_spec_conditional() {
        let spec: DecisionSpec =
            serde_json::from_str(r#"{"if":"writable","then":"allow","else":"deny"}"#).unwrap();
        assert!(matches!(spec.node, DecisionNode::Conditional(_)));
        assert!(!spec.force);
    }

    #[test]
    fn deserialize_decision_spec_conditional_with_force() {
        let spec: DecisionSpec = serde_json::from_str(
            r#"{"decision":{"if":"writable","then":"allow","else":"deny"},"force":true}"#,
        )
        .unwrap();
        assert!(matches!(spec.node, DecisionNode::Conditional(_)));
        assert!(spec.force);
    }

    #[test]
    fn deserialize_decision_spec_default_is_pattern_false() {
        let spec: DecisionSpec = serde_json::from_str(r#""allow""#).unwrap();
        assert!(!spec.is_pattern);

        let spec: DecisionSpec =
            serde_json::from_str(r#"{"decision":"deny","force":true}"#).unwrap();
        assert!(!spec.is_pattern);
    }

    #[test]
    fn deserialize_decision_spec_is_pattern_true() {
        let spec: DecisionSpec =
            serde_json::from_str(r#"{"decision":"deny","isPattern":true}"#).unwrap();
        assert!(matches!(spec.node, DecisionNode::Static(Decision::Deny)));
        assert!(!spec.force);
        assert!(spec.is_pattern);
    }

    #[test]
    fn deserialize_decision_spec_is_pattern_with_force() {
        let spec: DecisionSpec =
            serde_json::from_str(r#"{"decision":"deny","force":true,"isPattern":true}"#).unwrap();
        assert!(spec.force);
        assert!(spec.is_pattern);
    }

    #[test]
    fn deserialize_option_entry_with_check_file() {
        let entry: OptionEntry = serde_json::from_str(
            r#"{
                "decision": "allow",
                "checkFile": {
                    "values": {
                        "\\bsystem\\(": { "decision": "ask", "isPattern": true },
                        "*": "allow"
                    }
                }
            }"#,
        )
        .unwrap();
        let check = entry.check_file.expect("checkFile should be present");
        assert_eq!(check.on_unreadable, Decision::Deny);
        assert!(check.values.entries.contains_key("\\bsystem\\("));
        assert!(check.values.wildcard().is_some());
    }

    #[test]
    fn deserialize_check_file_on_unreadable_override() {
        let entry: OptionEntry = serde_json::from_str(
            r#"{
                "decision": "allow",
                "checkFile": {
                    "values": { "*": "allow" },
                    "onUnreadable": "ask"
                }
            }"#,
        )
        .unwrap();
        let check = entry.check_file.expect("checkFile should be present");
        assert_eq!(check.on_unreadable, Decision::Ask);
    }

    #[test]
    fn deserialize_option_entry_without_check_file() {
        let entry: OptionEntry =
            serde_json::from_str(r#"{"decision":"allow","values":{"*":"allow"}}"#).unwrap();
        assert!(entry.check_file.is_none());
    }

    #[test]
    fn deserialize_force_wrapper_flag() {
        let rules = test_rules();
        let bash = bash_rules(&rules);
        // command has -v with force=true
        let cmd = &bash.commands["command"];
        let flags = cmd.flags.as_ref().unwrap();
        let v_flag = lookup_with_alias("-v", flags).unwrap();
        assert!(matches!(v_flag.kind, FlagKind::Decision(Decision::Allow)));
        assert!(v_flag.force);
    }
}
