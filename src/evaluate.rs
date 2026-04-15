use crate::{
    bash::ExtractedCommand,
    decision::{stricter, Decision, DecisionNode, EvalResult, PathCheck},
    rules::{
        lookup_with_alias, BashRules, CommandNode, FlagEntry, FlagKind, OptionEntry, PositionalDef,
        WildcardMap,
    },
};

/// A decision paired with the reason it was made.
#[derive(Clone)]
struct Judgment {
    decision: Decision,
    force: bool,
    reason: String,
}

impl Judgment {
    fn new(decision: Decision, reason: String) -> Self {
        Self {
            decision,
            force: false,
            reason,
        }
    }
}

/// Merge judgments.  Forced judgments take priority.  If multiple forced
/// judgments disagree, the result is Ask (conflict).  Otherwise the strictest
/// non-forced decision wins (deny > ask > allow).  Empty defaults to ask.
fn merge_judgments(judgments: &[Judgment]) -> Judgment {
    let forced: Vec<&Judgment> = judgments.iter().filter(|j| j.force).collect();

    if !forced.is_empty() {
        let first = forced[0].decision;
        if forced.iter().all(|j| j.decision == first) {
            return forced[0].clone();
        }
        return Judgment {
            decision: Decision::Ask,
            force: false,
            reason: "conflicting force decisions".into(),
        };
    }

    judgments
        .iter()
        .cloned()
        .reduce(|a, b| {
            if stricter(a.decision, b.decision) == b.decision && b.decision != a.decision {
                b
            } else {
                a
            }
        })
        .unwrap_or(Judgment {
            decision: Decision::Ask,
            force: false,
            reason: "no decisions collected".into(),
        })
}

/// Split `--option=value` into `("--option", Some("value"))`.
/// Returns `None` if the arg doesn't contain `=` after a leading `-`.
fn split_option_eq(arg: &str) -> Option<(&str, &str)> {
    if arg.starts_with('-') {
        arg.split_once('=')
    } else {
        None
    }
}

/// Look up an option by name, trying `--option=value` splitting if the direct
/// lookup fails. Returns the matched entry and the extracted value.
fn lookup_option_with_eq<'a>(
    arg: &'a str,
    options: &'a WildcardMap<OptionEntry>,
) -> Option<(&'a OptionEntry, &'a str)> {
    // Try direct match first (space-separated form: --output /tmp/file)
    if let Some(entry) = lookup_with_alias(arg, options) {
        return Some((entry, ""));
    }
    // Try splitting on '=' (--output=/tmp/file)
    if let Some((name, value)) = split_option_eq(arg) {
        if let Some(entry) = lookup_with_alias(name, options) {
            return Some((entry, value));
        }
    }
    None
}

/// Positional arg with its original index in the args array.
struct Positional {
    index: usize,
    value: String,
}

/// Extract positional (non-flag) args, skipping flags and known option+value pairs.
fn extract_positionals(
    args: &[String],
    options: Option<&WildcardMap<OptionEntry>>,
) -> Vec<Positional> {
    let mut result = Vec::new();
    let mut i = 1; // skip command name at [0]
    while i < args.len() {
        if args[i].starts_with('-') {
            if let Some(opts) = options {
                if lookup_with_alias(&args[i], opts).is_some() {
                    i += 2; // skip option + its value
                    continue;
                }
            }
            i += 1; // skip --flag or --option=<value>
        } else {
            result.push(Positional {
                index: i,
                value: args[i].clone(),
            });
            i += 1;
        }
    }
    result
}

/// Format args as a readable command string for reasons.
fn fmt_cmd(args: &[String]) -> String {
    args.join(" ")
}

/// Result of wrapper stripping.
pub struct StrippedCommand {
    pub args: Vec<String>,
    pub expansion_flags: Vec<bool>,
    pub force_allow: bool,
}

/// Strip all chained wrapper commands from the front of an args list.
pub fn strip_wrappers(cmd: &ExtractedCommand, rules: &BashRules) -> StrippedCommand {
    let mut args = cmd.args.clone();
    let mut efl = cmd.expansion_flags.clone();
    let mut force_allow = false;

    loop {
        if force_allow || args.is_empty() {
            break;
        }
        let first = &args[0];
        let node = match rules.commands.get(first) {
            Some(n) if n.is_wrapper => n,
            _ => break,
        };

        // Remove the wrapper command name itself.
        args.remove(0);
        efl.remove(0);

        // Check for force flags first.
        if !args.is_empty() {
            if let Some(flags) = &node.flags {
                let head = &args[0];
                let has_force = flags.entries.iter().any(|(key, entry)| {
                    entry.force && (key == head || entry.aliases.iter().any(|a| a == head))
                });
                if has_force {
                    force_allow = true;
                    continue;
                }
            }
        }

        // Consume flags and options from the front.
        loop {
            if args.is_empty() {
                break;
            }
            let head = &args[0];
            if head == "--" {
                args.remove(0);
                efl.remove(0);
                break;
            }
            if !head.starts_with('-') {
                break;
            }
            if let Some(flags) = &node.flags {
                if lookup_with_alias(head, flags).is_some() {
                    args.remove(0);
                    efl.remove(0);
                    continue;
                }
            }
            if let Some(options) = &node.options {
                if lookup_with_alias(head, options).is_some() {
                    // Space-separated: consume option + value
                    args.remove(0);
                    efl.remove(0);
                    if !args.is_empty() {
                        args.remove(0);
                        efl.remove(0);
                    }
                    continue;
                }
                // --option=value: value is embedded, consume only this one arg
                if split_option_eq(head)
                    .is_some_and(|(name, _)| lookup_with_alias(name, options).is_some())
                {
                    args.remove(0);
                    efl.remove(0);
                    continue;
                }
            }
            // Unknown flag — stop consuming.
            break;
        }

        // Skip positional args (e.g., timeout's duration).
        for _ in 0..node.skip_positional {
            if !args.is_empty() {
                args.remove(0);
                efl.remove(0);
            }
        }
    }

    StrippedCommand {
        args,
        expansion_flags: efl,
        force_allow,
    }
}

/// Evaluate a single command against the rules.
pub fn evaluate_command(
    args: &[String],
    expansion_flags: &[bool],
    has_non_literal: bool,
    force_allow: bool,
    rules: &BashRules,
    cwd: &str,
) -> EvalResult {
    if force_allow {
        return EvalResult::Decided {
            decision: Decision::Allow,
            reason: format!("'{}': wrapper command force-allow", fmt_cmd(args)),
        };
    }

    if args.is_empty() {
        return EvalResult::Decided {
            decision: Decision::Ask,
            reason: "empty command".into(),
        };
    }

    // --help / --version is always allowed.
    if args.len() == 2 && (args[1] == "--help" || args[1] == "--version") {
        return EvalResult::Decided {
            decision: Decision::Allow,
            reason: format!("'{} {}' is always allowed", args[0], args[1]),
        };
    }

    let cmd = &args[0];
    let original = fmt_cmd(args);
    match rules.commands.get(cmd.as_str()) {
        None => EvalResult::Decided {
            decision: Decision::Ask,
            reason: format!("'{cmd}': unknown command"),
        },
        Some(node) => evaluate_node(
            args,
            expansion_flags,
            has_non_literal,
            node,
            &original,
            &format!("command '{cmd}'"),
            cwd,
        ),
    }
}

/// Evaluate a command's args against a node in the decision tree.
///
/// Evaluation order:
/// 1. Resolve subcmds — find first positional arg naming a subcmd, recurse.
/// 2. Collect pre-subcmd flag/option decisions from the parent node.
/// 3. At leaf: collect flag, option, expansion, args, positional decisions.
/// 4. Merge all string decisions (deny > ask > allow).
/// 5. If any positional decisions are path-conditional, emit CheckPaths.
fn evaluate_node(
    args: &[String],
    expansion_flags: &[bool],
    has_non_literal: bool,
    node: &CommandNode,
    original_cmd: &str,
    matched_rule: &str,
    cwd: &str,
) -> EvalResult {
    let cmd_str = original_cmd;

    // --- Subcmd resolution ---
    if let Some(subcmds) = &node.subcmds {
        let positionals = extract_positionals(args, node.options.as_ref());
        let subcmd_idx = positionals.first().map(|p| p.index);

        if let Some(idx) = subcmd_idx {
            let subcmd_name = &args[idx];
            if let Some(subcmd_node) = subcmds.entries.get(subcmd_name.as_str()) {
                // Collect pre-subcmd flag/option decisions.
                let (pre_judgments, pre_path_checks) =
                    collect_pre_subcmd_decisions(args, idx, node, original_cmd);

                // Build remaining args: [cmd_name] + args after subcmd.
                let mut remaining_args = vec![args[0].clone()];
                remaining_args.extend_from_slice(&args[idx + 1..]);
                let mut remaining_efl = vec![expansion_flags[0]];
                remaining_efl.extend_from_slice(&expansion_flags[idx + 1..]);

                let subcmd_eval = evaluate_node(
                    &remaining_args,
                    &remaining_efl,
                    has_non_literal,
                    subcmd_node,
                    original_cmd,
                    &format!("subcmd '{subcmd_name}'"),
                    cwd,
                );

                if pre_judgments.is_empty() && pre_path_checks.is_empty() {
                    return subcmd_eval;
                }
                // Merge pre-subcmd decisions with subcmd result.
                return match subcmd_eval {
                    EvalResult::Decided { decision, reason } => {
                        let mut all = pre_judgments.clone();
                        all.push(Judgment::new(decision, reason.clone()));
                        let merged = merge_judgments(&all);
                        if pre_path_checks.is_empty() {
                            EvalResult::Decided {
                                decision: merged.decision,
                                reason: merged.reason,
                            }
                        } else {
                            EvalResult::CheckPaths {
                                base_decision: merged.decision,
                                path_checks: pre_path_checks,
                                reason: merged.reason,
                            }
                        }
                    }
                    EvalResult::CheckPaths {
                        base_decision,
                        mut path_checks,
                        reason,
                    } => {
                        let mut all = pre_judgments.clone();
                        all.push(Judgment::new(base_decision, reason.clone()));
                        let merged = merge_judgments(&all);
                        path_checks.extend(pre_path_checks);
                        EvalResult::CheckPaths {
                            base_decision: merged.decision,
                            path_checks,
                            reason: merged.reason,
                        }
                    }
                };
            } else if let Some(wildcard) = subcmds.wildcard() {
                return EvalResult::Decided {
                    decision: wildcard.decision.unwrap_or(Decision::Ask),
                    reason: format!("'{cmd_str}': subcmd '{subcmd_name}' matched wildcard"),
                };
            } else {
                return EvalResult::Decided {
                    decision: Decision::Ask,
                    reason: format!("'{cmd_str}': subcmd '{subcmd_name}' not in allowed list"),
                };
            }
        }
        // No subcmd found (bare command) — fall through to leaf evaluation.
    }

    // --- Leaf node: collect decisions from flags, options, args, positional ---

    let mut judgments: Vec<Judgment> = Vec::new();
    let mut path_checks: Vec<PathCheck> = Vec::new();
    let pos_args = extract_positionals(args, node.options.as_ref());

    // Flags
    if let Some(flags) = &node.flags {
        for arg in &args[1..] {
            if !arg.starts_with('-') {
                continue;
            }
            let matched = lookup_with_alias(arg, flags).or(flags.wildcard());
            if let Some(entry) = matched {
                collect_flag_decisions(
                    entry,
                    arg,
                    cmd_str,
                    &pos_args,
                    &mut judgments,
                    &mut path_checks,
                );
            } else if node
                .options
                .as_ref()
                .is_some_and(|opts| lookup_option_with_eq(arg, opts).is_some())
            {
                // This dash-arg is a known option (flag-with-value); the
                // options loop below will handle it — skip it here so we
                // don't report it as an unknown flag.
            } else {
                judgments.push(Judgment::new(
                    Decision::Ask,
                    format!("'{cmd_str}': unknown flag '{arg}'"),
                ));
            }
        }
    }

    // Options (flags that take values)
    if let Some(options) = &node.options {
        let mut i = 1;
        while i < args.len() {
            if args[i].starts_with('-') {
                if let Some(entry) = lookup_with_alias(&args[i], options) {
                    // Space-separated form: --output /tmp/file
                    let value = args.get(i + 1).map(|s| s.as_str()).unwrap_or("");
                    evaluate_option_value(
                        entry,
                        &args[i],
                        value,
                        cmd_str,
                        &mut judgments,
                        &mut path_checks,
                    );
                } else if let Some((name, eq_value)) = split_option_eq(&args[i]) {
                    if let Some(entry) = lookup_with_alias(name, options) {
                        // Equals form: --output=/tmp/file
                        evaluate_option_value(
                            entry,
                            name,
                            eq_value,
                            cmd_str,
                            &mut judgments,
                            &mut path_checks,
                        );
                    }
                }
            }
            i += 1;
        }
    }

    // Expansion coverage check
    if has_non_literal && !check_expansion_coverage(args, expansion_flags, node) {
        judgments.push(Judgment::new(
            Decision::Ask,
            format!("'{cmd_str}': contains uncovered variable expansions"),
        ));
    }

    // Positional (ordered, keyed by count)
    let (positional_judgments, positional_path_checks) = if let Some(positional) = &node.positional
    {
        collect_positional_decisions(positional, &pos_args, cmd_str, "positional")
    } else {
        (Vec::new(), Vec::new())
    };
    path_checks.extend(positional_path_checks);

    // cwdCheck — conditional decision applied to the working directory
    if let Some(cwd_node) = &node.cwd_check {
        classify_positional_entry(
            cwd_node,
            cwd,
            cmd_str,
            "cwdCheck",
            &mut judgments,
            &mut path_checks,
        );
    }

    // --- Merge ---
    let all_judgments: Vec<Judgment> = judgments
        .iter()
        .chain(positional_judgments.iter())
        .cloned()
        .collect();

    if !path_checks.is_empty() {
        let base = if all_judgments.is_empty() {
            Judgment::new(Decision::Allow, format!("'{cmd_str}': allowed"))
        } else {
            merge_judgments(&all_judgments)
        };
        EvalResult::CheckPaths {
            base_decision: base.decision,
            path_checks,
            reason: base.reason,
        }
    } else if !all_judgments.is_empty() {
        let merged = merge_judgments(&all_judgments);
        EvalResult::Decided {
            decision: merged.decision,
            reason: merged.reason,
        }
    } else {
        let base = node.decision.unwrap_or(Decision::Ask);
        EvalResult::Decided {
            decision: base,
            reason: format!("'{}': {} {}", cmd_str, matched_rule, base.description()),
        }
    }
}

/// Collect judgments and path checks from a matched flag entry.
fn collect_flag_decisions(
    entry: &FlagEntry,
    arg: &str,
    cmd_str: &str,
    pos_args: &[Positional],
    judgments: &mut Vec<Judgment>,
    path_checks: &mut Vec<PathCheck>,
) {
    match &entry.kind {
        FlagKind::Decision(d) => {
            judgments.push(Judgment {
                decision: *d,
                force: entry.force,
                reason: format!("'{cmd_str}': flag '{arg}' {}", d.description()),
            });
        }
        FlagKind::Positional(pos_defs) => {
            let context = format!("flag '{arg}' positional");
            let (pj, pc) = collect_positional_decisions(pos_defs, pos_args, cmd_str, &context);
            judgments.extend(pj);
            path_checks.extend(pc);
        }
    }
}

/// Classify a single positional entry as either a Judgment or PathCheck.
fn classify_positional_entry(
    entry: &DecisionNode,
    path_val: &str,
    cmd_str: &str,
    context: &str,
    judgments: &mut Vec<Judgment>,
    path_checks: &mut Vec<PathCheck>,
) {
    match entry {
        DecisionNode::Static(d) => {
            judgments.push(Judgment::new(
                *d,
                format!(
                    "'{}': {} arg '{}' {}",
                    cmd_str,
                    context,
                    path_val,
                    d.description()
                ),
            ));
        }
        DecisionNode::Conditional(_) => {
            path_checks.push(PathCheck {
                path: path_val.to_string(),
                decision: entry.clone(),
                force: false,
            });
        }
    }
}

/// Evaluate positional rules against positional args, collecting judgments and path checks.
fn collect_positional_decisions(
    pos_defs: &WildcardMap<PositionalDef>,
    pos_args: &[Positional],
    cmd_str: &str,
    context: &str,
) -> (Vec<Judgment>, Vec<PathCheck>) {
    let mut judgments = Vec::new();
    let mut path_checks = Vec::new();

    let count = pos_args.len().to_string();
    let (pos_def, is_wildcard) = match pos_defs.entries.get(&count) {
        Some(def) => (Some(def), false),
        None => (pos_defs.wildcard(), true),
    };

    match pos_def {
        Some(PositionalDef::Array(entries)) => {
            for (i, entry) in entries.iter().enumerate() {
                let path_val = pos_args.get(i).map(|p| p.value.as_str()).unwrap_or("");
                classify_positional_entry(
                    entry,
                    path_val,
                    cmd_str,
                    context,
                    &mut judgments,
                    &mut path_checks,
                );
            }
        }
        Some(PositionalDef::Single(entry)) => {
            let targets = if is_wildcard {
                pos_args
            } else {
                &pos_args[..1.min(pos_args.len())]
            };
            for pos in targets {
                classify_positional_entry(
                    entry,
                    &pos.value,
                    cmd_str,
                    context,
                    &mut judgments,
                    &mut path_checks,
                );
            }
        }
        None => {
            judgments.push(Judgment::new(
                Decision::Ask,
                format!(
                    "'{}': {} {count} positional args not in allowed counts",
                    cmd_str, context
                ),
            ));
        }
    }

    (judgments, path_checks)
}

/// Evaluate an option entry's value, pushing either a `Judgment` or `PathCheck`.
fn evaluate_option_value(
    entry: &OptionEntry,
    option_name: &str,
    value: &str,
    cmd_str: &str,
    judgments: &mut Vec<Judgment>,
    path_checks: &mut Vec<PathCheck>,
) {
    if let Some(values) = &entry.values {
        let (spec, is_wildcard) = match values.entries.get(value) {
            Some(s) => (Some(s), false),
            None => (values.wildcard(), true),
        };
        if let Some(spec) = spec {
            match &spec.node {
                DecisionNode::Static(d) => {
                    let reason = if is_wildcard {
                        format!(
                            "'{}': option '{}' value '{}' matched wildcard",
                            cmd_str, option_name, value
                        )
                    } else {
                        format!(
                            "'{}': option '{}' value '{}' {}",
                            cmd_str,
                            option_name,
                            value,
                            d.description()
                        )
                    };
                    judgments.push(Judgment {
                        decision: *d,
                        force: spec.force,
                        reason,
                    });
                }
                DecisionNode::Conditional(_) => {
                    path_checks.push(PathCheck {
                        path: value.to_string(),
                        decision: spec.node.clone(),
                        force: spec.force,
                    });
                }
            }
        } else {
            judgments.push(Judgment::new(
                Decision::Ask,
                format!(
                    "'{}': option '{}' value '{}' not in allowed list",
                    cmd_str, option_name, value
                ),
            ));
        }
    } else {
        judgments.push(Judgment {
            decision: entry.decision,
            force: entry.force,
            reason: format!(
                "'{}': option '{}' {}",
                cmd_str,
                option_name,
                entry.decision.description()
            ),
        });
    }
}

/// Collect flag/option decisions from args between the command name and the subcmd.
fn collect_pre_subcmd_decisions(
    args: &[String],
    subcmd_idx: usize,
    node: &CommandNode,
    original_cmd: &str,
) -> (Vec<Judgment>, Vec<PathCheck>) {
    let cmd_str = original_cmd;
    let mut judgments = Vec::new();
    let mut path_checks = Vec::new();
    let mut i = 1;
    while i < subcmd_idx {
        let arg = &args[i];
        if !arg.starts_with('-') {
            i += 1;
            continue;
        }
        let mut found = false;
        if let Some(flags) = &node.flags {
            let matched = lookup_with_alias(arg, flags).or(flags.wildcard());
            if let Some(entry) = matched {
                if let FlagKind::Decision(d) = &entry.kind {
                    judgments.push(Judgment {
                        decision: *d,
                        force: entry.force,
                        reason: format!("'{}': flag '{}' {}", cmd_str, arg, d.description()),
                    });
                }
                // FlagKind::Positional is not evaluated here — pre-subcmd
                // flags don't have positional args to overlay onto.
                found = true;
            }
        }
        if !found {
            if let Some(options) = &node.options {
                if let Some(entry) = lookup_with_alias(arg, options) {
                    // Space-separated form: --option value
                    let value = args.get(i + 1).map(|s| s.as_str()).unwrap_or("");
                    evaluate_option_value(
                        entry,
                        arg,
                        value,
                        cmd_str,
                        &mut judgments,
                        &mut path_checks,
                    );
                    found = true;
                    i += 1; // skip the option's value
                } else if let Some((name, eq_value)) = split_option_eq(arg) {
                    if let Some(entry) = lookup_with_alias(name, options) {
                        // Equals form: --option=value
                        evaluate_option_value(
                            entry,
                            name,
                            eq_value,
                            cmd_str,
                            &mut judgments,
                            &mut path_checks,
                        );
                        found = true;
                        // No i += 1 — value is embedded in this arg
                    }
                }
            }
        }
        if !found && (node.flags.is_some() || node.options.is_some()) {
            judgments.push(Judgment::new(
                Decision::Ask,
                format!("'{cmd_str}': unknown flag/option '{arg}'"),
            ));
        }
        i += 1;
    }
    (judgments, path_checks)
}

/// Check if all non-literal args are covered by option-level or node-level allow_expansions.
#[cfg(test)]
pub fn check_expansion_coverage_pub(
    args: &[String],
    expansion_flags: &[bool],
    node: &CommandNode,
) -> bool {
    check_expansion_coverage(args, expansion_flags, node)
}

fn check_expansion_coverage(args: &[String], expansion_flags: &[bool], node: &CommandNode) -> bool {
    let mut covered: Vec<usize> = Vec::new();
    if let Some(options) = &node.options {
        for (i, arg) in args.iter().enumerate().skip(1) {
            if arg.starts_with('-') {
                let entry = lookup_with_alias(arg, options).or_else(|| {
                    split_option_eq(arg).and_then(|(name, _)| lookup_with_alias(name, options))
                });
                if let Some(entry) = entry {
                    if entry.allow_expansions {
                        // For --option=value form, the expansion is in the
                        // same arg (index i), not the next one.
                        if split_option_eq(arg).is_some() {
                            covered.push(i);
                        } else {
                            covered.push(i + 1);
                        }
                    }
                }
            }
        }
    }

    for (i, &has_expansion) in expansion_flags.iter().enumerate() {
        if has_expansion && !covered.contains(&i) && !node.allow_expansions {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bash::ExtractedCommand;

    fn test_rules() -> crate::rules::Rules {
        let json = include_str!("../tests/fixtures/test_rules.json");
        serde_json::from_str(json).expect("test rules should parse")
    }

    fn test_bash_rules(rules: &crate::rules::Rules) -> &BashRules {
        rules.tools.bash.as_ref().expect("Bash rules should exist")
    }

    /// Helper: build args and uniform expansion_flags (all literal).
    fn args(strs: &[&str]) -> (Vec<String>, Vec<bool>) {
        let a: Vec<String> = strs.iter().map(|s| s.to_string()).collect();
        let e = vec![false; a.len()];
        (a, e)
    }

    const TEST_CWD: &str = "/test/cwd";

    /// Helper: evaluate a literal command (no expansions).
    fn eval(strs: &[&str]) -> EvalResult {
        let rules = test_rules();
        let bash = test_bash_rules(&rules);
        let (a, e) = args(strs);
        evaluate_command(&a, &e, false, false, bash, TEST_CWD)
    }

    /// Helper: extract decision from EvalResult.
    fn decision(result: &EvalResult) -> Decision {
        match result {
            EvalResult::Decided { decision, .. } => *decision,
            EvalResult::CheckPaths { base_decision, .. } => *base_decision,
        }
    }

    // --- evaluate_command basics ---

    #[test]
    fn empty_args_ask() {
        let rules = test_rules();
        let bash = test_bash_rules(&rules);
        let result = evaluate_command(&[], &[], false, false, bash, TEST_CWD);
        assert_eq!(decision(&result), Decision::Ask);
    }

    #[test]
    fn unknown_command_ask() {
        let result = eval(&["totally-unknown-cmd"]);
        assert_eq!(decision(&result), Decision::Ask);
    }

    #[test]
    fn help_flag_always_allow() {
        // dd is deny, but --help always overrides to allow
        let result = eval(&["dd", "--help"]);
        assert_eq!(decision(&result), Decision::Allow);
    }

    #[test]
    fn version_flag_always_allow() {
        // dd is deny, but --version always overrides to allow
        let result = eval(&["dd", "--version"]);
        assert_eq!(decision(&result), Decision::Allow);
    }

    #[test]
    fn force_allow_overrides() {
        let rules = test_rules();
        let bash = test_bash_rules(&rules);
        let (a, e) = args(&["anything"]);
        let result = evaluate_command(&a, &e, false, true, bash, TEST_CWD);
        assert_eq!(decision(&result), Decision::Allow);
    }

    // --- Simple decisions ---

    #[test]
    fn simple_allow() {
        // ls is a simple "allow" command
        let result = eval(&["ls"]);
        assert_eq!(decision(&result), Decision::Allow);
    }

    #[test]
    fn simple_deny() {
        // dd is a simple "deny" command
        let result = eval(&["dd"]);
        assert_eq!(decision(&result), Decision::Deny);
    }

    #[test]
    fn simple_ask() {
        // sh has decision "ask" with flags defined
        let result = eval(&["sh"]);
        assert_eq!(decision(&result), Decision::Ask);
    }

    // --- Subcmds ---

    #[test]
    fn subcmd_allow() {
        // cargo test → subcmd "test" is allow
        let result = eval(&["cargo", "test"]);
        assert_eq!(decision(&result), Decision::Allow);
    }

    #[test]
    fn subcmd_deny() {
        // git push → subcmd "push" is deny
        let result = eval(&["git", "push"]);
        assert_eq!(decision(&result), Decision::Deny);
    }

    #[test]
    fn subcmd_wildcard() {
        // npm audit → not in named subcmds, matches "*" wildcard (ask)
        let result = eval(&["npm", "audit"]);
        assert_eq!(decision(&result), Decision::Ask);
    }

    #[test]
    fn subcmd_not_listed_no_wildcard() {
        // nix flake has subcmds (check, show) but no wildcard;
        // "nix flake delete" is not in the list → ask
        let result = eval(&["nix", "flake", "delete"]);
        assert_eq!(decision(&result), Decision::Ask);
    }

    #[test]
    fn nested_subcmd_allow() {
        // nix flake check → nested subcmd "check" is allow
        let result = eval(&["nix", "flake", "check"]);
        assert_eq!(decision(&result), Decision::Allow);
    }

    #[test]
    fn nested_subcmd_deny() {
        // git clean → subcmd "clean" is deny (single-level, but tests deny path)
        // For a true nested deny, we don't have one in the fixture, so test
        // the single-level deny subcmd path via git clean.
        let result = eval(&["git", "clean"]);
        assert_eq!(decision(&result), Decision::Deny);
    }

    #[test]
    fn bare_command_with_subcmds_uses_node_decision() {
        // "cargo" with no subcmd arg — falls through to node decision "ask"
        let result = eval(&["cargo"]);
        assert_eq!(decision(&result), Decision::Ask);
    }

    // --- Flags ---

    #[test]
    fn flag_allow() {
        // make -j → flag "-j" is allow
        let result = eval(&["make", "-j"]);
        assert_eq!(decision(&result), Decision::Allow);
    }

    #[test]
    fn flag_deny() {
        // rm -r → flag "-r" is deny
        let result = eval(&["rm", "-r"]);
        assert_eq!(decision(&result), Decision::Deny);
    }

    #[test]
    fn flag_alias() {
        // make -n → alias for --dry-run, which is allow
        let result = eval(&["make", "-n"]);
        assert_eq!(decision(&result), Decision::Allow);
    }

    #[test]
    fn flag_wildcard() {
        // make --unknown-flag → not in named flags, matches "*" wildcard (allow)
        let result = eval(&["make", "--unknown-flag"]);
        assert_eq!(decision(&result), Decision::Allow);
    }

    #[test]
    fn flag_deny_overrides_allow() {
        // rm has decision allow, but -r flag is deny → merged to deny
        let result = eval(&["rm", "-r", "/tmp/file.txt"]);
        assert_eq!(decision(&result), Decision::Deny);
    }

    // --- Pre-subcmd flags/options ---

    #[test]
    fn pre_subcmd_option_conditional_value() {
        // git -C /tmp/foo status → -C value is evaluated as a conditional
        // path check, not just the option's base decision. The path check
        // is returned alongside the subcmd's decision for resolution by
        // the caller.
        let result = eval(&["git", "-C", "/tmp/foo", "status"]);
        assert!(matches!(result, EvalResult::CheckPaths { .. }));
        let EvalResult::CheckPaths {
            base_decision,
            path_checks,
            ..
        } = &result
        else {
            panic!("expected CheckPaths");
        };
        assert_eq!(*base_decision, Decision::Allow);
        assert_eq!(path_checks.len(), 1);
        assert_eq!(path_checks[0].path, "/tmp/foo");
    }

    #[test]
    fn pre_subcmd_unknown_flag_asks() {
        // git has options defined (-C) but no flags; an unknown flag/option
        // like --unknown before a subcmd hits the unknown pre-subcmd path → ask
        let result = eval(&["git", "--unknown", "status"]);
        assert_eq!(decision(&result), Decision::Ask);
    }

    // --- Options ---

    #[test]
    fn option_with_value_allow() {
        // curl --request GET → value "GET" is allow
        let result = eval(&["curl", "--request", "GET"]);
        assert_eq!(decision(&result), Decision::Allow);
    }

    #[test]
    fn option_with_value_deny() {
        // curl --request DELETE → value "DELETE" is deny
        let result = eval(&["curl", "--request", "DELETE"]);
        assert_eq!(decision(&result), Decision::Deny);
    }

    #[test]
    fn option_with_value_wildcard() {
        // curl --request POST → not in named values, matches "*" wildcard (ask)
        let result = eval(&["curl", "--request", "POST"]);
        assert_eq!(decision(&result), Decision::Ask);
    }

    #[test]
    fn option_alias() {
        // curl -X GET → -X is alias for --request, value "GET" is allow
        let result = eval(&["curl", "-X", "GET"]);
        assert_eq!(decision(&result), Decision::Allow);
    }

    #[test]
    fn option_without_values_dict() {
        // timeout -s TERM → -s has no values dict, just a decision (allow)
        let result = eval(&["timeout", "-s", "TERM"]);
        assert_eq!(decision(&result), Decision::Allow);
    }

    #[test]
    fn option_value_conditional_returns_check_paths() {
        // curl --output /tmp/out.txt → value has a writable conditional
        let result = eval(&["curl", "--output", "/tmp/out.txt"]);
        match &result {
            EvalResult::CheckPaths { path_checks, .. } => {
                assert_eq!(path_checks.len(), 1);
                assert_eq!(path_checks[0].path, "/tmp/out.txt");
                assert!(!path_checks[0].force);
            }
            _ => panic!("expected CheckPaths, got Decided"),
        }
    }

    #[test]
    fn unknown_option_silently_skipped() {
        // curl has `options` defined but --unknown-flag is not in it.
        // Unknown options are not matched and produce no judgment, so the
        // node's base decision (ask) wins.
        let result = eval(&["curl", "--unknown-flag", "value"]);
        assert_eq!(decision(&result), Decision::Ask);
    }

    // --- Expansion coverage ---

    #[test]
    fn expansion_uncovered_asks() {
        let rules = test_rules();
        let bash = test_bash_rules(&rules);
        // ls has no allow_expansions — expansion in arg is uncovered
        let a: Vec<String> = vec!["ls".into(), "".into()];
        let e = vec![false, true]; // second arg is non-literal
        let result = evaluate_command(&a, &e, true, false, bash, TEST_CWD);
        assert_eq!(decision(&result), Decision::Ask);
    }

    #[test]
    fn expansion_covered_by_node_allow_expansions() {
        let rules = test_rules();
        let bash = test_bash_rules(&rules);
        // env has allowExpansions: true at node level
        let a: Vec<String> = vec!["env".into(), "".into()];
        let e = vec![false, true];
        let result = evaluate_command(&a, &e, true, false, bash, TEST_CWD);
        assert_eq!(decision(&result), Decision::Allow);
    }

    #[test]
    fn expansion_covered_by_option_allow_expansions() {
        let rules = test_rules();
        let bash = test_bash_rules(&rules);
        // git commit -m has allowExpansions: true on the option
        let a: Vec<String> = vec![
            "git".into(),
            "commit".into(),
            "-m".into(),
            "".into(), // expansion in value position
        ];
        let e = vec![false, false, false, true];
        let result = evaluate_command(&a, &e, true, false, bash, TEST_CWD);
        assert_eq!(decision(&result), Decision::Allow);
    }

    #[test]
    fn check_expansion_coverage_direct() {
        let rules = test_rules();
        let bash = test_bash_rules(&rules);
        // git commit has -m with allowExpansions — test at the subcmd node level
        let commit_node = &bash.commands["git"].subcmds.as_ref().unwrap().entries["commit"];
        let a: Vec<String> = vec!["git".into(), "-m".into(), "".into()];
        let e = vec![false, false, true];
        assert!(check_expansion_coverage_pub(&a, &e, commit_node));
    }

    #[test]
    fn check_expansion_coverage_uncovered() {
        let rules = test_rules();
        let bash = test_bash_rules(&rules);
        // curl has options but none with allowExpansions — expansion is uncovered
        let node = &bash.commands["curl"];
        let a: Vec<String> = vec!["curl".into(), "".into()];
        let e = vec![false, true];
        assert!(!check_expansion_coverage_pub(&a, &e, node));
    }

    // --- Positional (count-based) ---

    #[test]
    fn positional_count_1_conditional() {
        // cd /tmp/foo → count "1" has a conditional (readable check)
        let result = eval(&["cd", "/tmp/foo"]);
        match &result {
            EvalResult::CheckPaths { path_checks, .. } => {
                assert_eq!(path_checks.len(), 1);
                assert_eq!(path_checks[0].path, "/tmp/foo");
            }
            _ => panic!("expected CheckPaths for cd positional, got Decided"),
        }
    }

    #[test]
    fn positional_count_mismatch_ask() {
        // cd /tmp/a /tmp/b → 2 positionals, no "2" key, no wildcard → ask
        let result = eval(&["cd", "/tmp/a", "/tmp/b"]);
        assert_eq!(decision(&result), Decision::Ask);
    }

    #[test]
    fn positional_count_missing_ask() {
        // cd a b c → 3 positionals, no "3" key, no wildcard → ask
        let result = eval(&["cd", "a", "b", "c"]);
        assert_eq!(decision(&result), Decision::Ask);
    }

    // --- Positional (wildcard count) ---

    #[test]
    fn positional_wildcard_count() {
        // sed 's/a/b/' file1 file2 file3 → "*" positional is allow, applies to all
        // (sed also has "1" with a conditional, but 3 args doesn't match "1",
        // so the wildcard is used and it's a static "allow")
        let result = eval(&["sed", "a", "b", "c"]);
        assert_eq!(decision(&result), Decision::Allow);
    }

    #[test]
    fn positional_wildcard_single_conditional_checks_all() {
        // cd has "1": conditional, no wildcard — 2 args → unmatched count → ask
        let result = eval(&["cd", "/tmp/a", "/tmp/b"]);
        assert_eq!(decision(&result), Decision::Ask);

        // patch has "*": Single(conditional readable) — wildcard applies to all positionals
        let result = eval(&["patch", "/tmp/a", "/tmp/b"]);
        match &result {
            EvalResult::CheckPaths { path_checks, .. } => {
                // Should have a readable check for EACH positional (/tmp/a and /tmp/b)
                assert_eq!(path_checks.len(), 2);
                assert_eq!(path_checks[0].path, "/tmp/a");
                assert_eq!(path_checks[1].path, "/tmp/b");
            }
            _ => panic!("expected CheckPaths, got Decided"),
        }
    }

    // --- Positional with path conditionals ---

    #[test]
    fn positional_conditional_returns_check_paths() {
        // cp /tmp/src.txt /tmp/dst.txt → count "2" array with [readable, writable]
        let result = eval(&["cp", "/tmp/src.txt", "/tmp/dst.txt"]);
        match &result {
            EvalResult::CheckPaths { path_checks, .. } => {
                assert_eq!(path_checks.len(), 2);
                assert_eq!(path_checks[0].path, "/tmp/src.txt");
                assert_eq!(path_checks[1].path, "/tmp/dst.txt");
            }
            _ => panic!("expected CheckPaths, got Decided"),
        }
    }

    // --- Wrapper stripping ---

    #[test]
    fn non_wrapper_passthrough() {
        let rules = test_rules();
        let bash = test_bash_rules(&rules);
        // ls is not a wrapper — passthrough unchanged
        let cmd = ExtractedCommand {
            args: vec!["ls".into(), "file.txt".into()],
            expansion_flags: vec![false, false],
        };
        let stripped = strip_wrappers(&cmd, bash);
        assert_eq!(stripped.args, vec!["ls", "file.txt"]);
        assert!(!stripped.force_allow);
    }

    #[test]
    fn wrapper_strips_command_and_skips_positional() {
        let rules = test_rules();
        let bash = test_bash_rules(&rules);
        // timeout is isWrapper with skipPositional=1
        // "timeout 5 ls file.txt" → strips "timeout", skips "5", leaves "ls file.txt"
        let cmd = ExtractedCommand {
            args: vec!["timeout".into(), "5".into(), "ls".into(), "file.txt".into()],
            expansion_flags: vec![false, false, false, false],
        };
        let stripped = strip_wrappers(&cmd, bash);
        assert_eq!(stripped.args, vec!["ls", "file.txt"]);
        assert!(!stripped.force_allow);
    }

    #[test]
    fn wrapper_with_flags() {
        let rules = test_rules();
        let bash = test_bash_rules(&rules);
        // timeout --foreground 5 ls → strips timeout, consumes --foreground, skips "5", leaves "ls"
        let cmd = ExtractedCommand {
            args: vec![
                "timeout".into(),
                "--foreground".into(),
                "5".into(),
                "ls".into(),
            ],
            expansion_flags: vec![false, false, false, false],
        };
        let stripped = strip_wrappers(&cmd, bash);
        assert_eq!(stripped.args, vec!["ls"]);
        assert!(!stripped.force_allow);
    }

    #[test]
    fn force_wrapper_sets_force_allow() {
        let rules = test_rules();
        let bash = test_bash_rules(&rules);
        // command -v dd → -v has force=true, sets force_allow
        let cmd = ExtractedCommand {
            args: vec!["command".into(), "-v".into(), "dd".into()],
            expansion_flags: vec![false, false, false],
        };
        let stripped = strip_wrappers(&cmd, bash);
        assert!(stripped.force_allow);
    }

    #[test]
    fn force_wrapper_alias() {
        let rules = test_rules();
        let bash = test_bash_rules(&rules);
        // command -V dd → -V also has force=true
        let cmd = ExtractedCommand {
            args: vec!["command".into(), "-V".into(), "dd".into()],
            expansion_flags: vec![false, false, false],
        };
        let stripped = strip_wrappers(&cmd, bash);
        assert!(stripped.force_allow);
    }

    #[test]
    fn chained_wrappers() {
        let rules = test_rules();
        let bash = test_bash_rules(&rules);
        // timeout 5 timeout 10 ls → strips both wrappers, leaves "ls"
        let cmd = ExtractedCommand {
            args: vec![
                "timeout".into(),
                "5".into(),
                "timeout".into(),
                "10".into(),
                "ls".into(),
            ],
            expansion_flags: vec![false, false, false, false, false],
        };
        let stripped = strip_wrappers(&cmd, bash);
        assert_eq!(stripped.args, vec!["ls"]);
    }

    #[test]
    fn wrapper_with_double_dash() {
        let rules = test_rules();
        let bash = test_bash_rules(&rules);
        // timeout -- 5 ls → "--" stops flag consumption, "5" is skip_positional, "ls" remains
        let cmd = ExtractedCommand {
            args: vec!["timeout".into(), "--".into(), "5".into(), "ls".into()],
            expansion_flags: vec![false, false, false, false],
        };
        let stripped = strip_wrappers(&cmd, bash);
        assert_eq!(stripped.args, vec!["ls"]);
    }

    // --- Flag positional overlay ---

    #[test]
    fn flag_positional_overlay_without_flag_returns_base_check() {
        // patch /tmp/file.txt — no -i flag, only base readable check from positional "*"
        let result = eval(&["patch", "/tmp/file.txt"]);
        match &result {
            EvalResult::CheckPaths { path_checks, .. } => {
                assert_eq!(path_checks.len(), 1, "only base positional check");
                assert_eq!(path_checks[0].path, "/tmp/file.txt");
            }
            _ => panic!("expected CheckPaths, got Decided"),
        }
    }

    #[test]
    fn flag_positional_overlay_with_flag_returns_both_checks() {
        // patch -i /tmp/file.txt → base readable (from positional "*") + overlay writable (from -i)
        let result = eval(&["patch", "-i", "/tmp/file.txt"]);
        match &result {
            EvalResult::CheckPaths { path_checks, .. } => {
                assert_eq!(path_checks.len(), 2, "base readable + overlay writable");
                assert!(path_checks.iter().all(|pc| pc.path == "/tmp/file.txt"));
            }
            _ => panic!("expected CheckPaths, got Decided"),
        }
    }

    #[test]
    fn flag_positional_overlay_alias() {
        // patch --in-place is not a defined alias for -i in the fixture,
        // but we can verify -i itself works (already tested above).
        // Instead, test that sed --in-place is an alias for sed -i (both ask).
        let result = eval(&["sed", "--in-place", "s/a/b/", "/tmp/file.txt"]);
        let result2 = eval(&["sed", "-i", "s/a/b/", "/tmp/file.txt"]);
        assert_eq!(decision(&result), decision(&result2));
    }

    #[test]
    fn flag_positional_deserialize() {
        let rules = test_rules();
        let bash = test_bash_rules(&rules);
        // patch has -i with FlagKind::Positional (writable overlay)
        let node = &bash.commands["patch"];
        let flags = node.flags.as_ref().unwrap();
        let entry = lookup_with_alias("-i", flags).unwrap();
        assert!(matches!(entry.kind, crate::rules::FlagKind::Positional(_)));
        assert!(!entry.force);
    }

    // --- cwdCheck ---

    #[test]
    fn cwd_check_produces_path_check() {
        // unzip has cwdCheck = ifWritable. When run from /tmp (writable cwd),
        // cwdCheck adds a PathCheck for the cwd path.
        let rules = test_rules();
        let bash = test_bash_rules(&rules);
        let (a, e) = args(&["unzip", "/tmp/archive.zip"]);
        let result = evaluate_command(&a, &e, false, false, bash, "/tmp/project");
        match &result {
            EvalResult::CheckPaths { path_checks, .. } => {
                // Should have path checks for: positional (archive readable) + cwdCheck (cwd writable)
                assert!(
                    path_checks.iter().any(|pc| pc.path == "/tmp/project"),
                    "expected a path check for the cwd"
                );
                assert!(
                    path_checks.iter().any(|pc| pc.path == "/tmp/archive.zip"),
                    "expected a path check for the archive"
                );
            }
            _ => panic!("expected CheckPaths, got Decided"),
        }
    }

    #[test]
    fn cwd_check_not_present_on_regular_command() {
        // ls has no cwdCheck — should not produce a cwd path check
        let result = eval(&["ls", "/tmp/foo"]);
        match &result {
            EvalResult::Decided { decision, .. } => {
                assert_eq!(*decision, Decision::Allow);
            }
            _ => panic!("expected Decided for ls, got CheckPaths"),
        }
    }

    #[test]
    fn cwd_check_with_read_only_flag_still_applies() {
        // unzip -l archive.zip: -l is allow, but cwdCheck still adds
        // a writable check on the cwd
        let rules = test_rules();
        let bash = test_bash_rules(&rules);
        let (a, e) = args(&["unzip", "-l", "/tmp/archive.zip"]);
        let result = evaluate_command(&a, &e, false, false, bash, "/tmp/project");
        match &result {
            EvalResult::CheckPaths { path_checks, .. } => {
                assert!(path_checks.iter().any(|pc| pc.path == "/tmp/project"));
            }
            _ => panic!("expected CheckPaths due to cwdCheck, got Decided"),
        }
    }

    // --- split_option_eq ---

    #[test]
    fn split_option_eq_long_option() {
        assert_eq!(
            split_option_eq("--output=/tmp/file"),
            Some(("--output", "/tmp/file"))
        );
    }

    #[test]
    fn split_option_eq_short_option() {
        assert_eq!(split_option_eq("-o=/tmp/file"), Some(("-o", "/tmp/file")));
    }

    #[test]
    fn split_option_eq_no_equals() {
        assert_eq!(split_option_eq("--output"), None);
    }

    #[test]
    fn split_option_eq_no_dash() {
        assert_eq!(split_option_eq("output=/tmp/file"), None);
    }

    #[test]
    fn split_option_eq_empty_value() {
        assert_eq!(split_option_eq("--output="), Some(("--output", "")));
    }

    #[test]
    fn split_option_eq_value_with_equals() {
        // git -c key=value — the first = is the split point
        assert_eq!(
            split_option_eq("--config=core.editor=vim"),
            Some(("--config", "core.editor=vim"))
        );
    }

    // --- option with = form in evaluate_command ---

    #[test]
    fn option_eq_form_produces_path_check() {
        let rules = test_rules();
        let bash = test_bash_rules(&rules);
        // curl --output=/tmp/file should produce a path check for /tmp/file
        let (a, e) = args(&["curl", "--request", "GET", "--output=/tmp/file"]);
        let result = evaluate_command(&a, &e, false, false, bash, "/tmp");
        match &result {
            EvalResult::CheckPaths { path_checks, .. } => {
                assert!(
                    path_checks.iter().any(|pc| pc.path == "/tmp/file"),
                    "expected path check for /tmp/file"
                );
            }
            _ => panic!("expected CheckPaths"),
        }
    }

    #[test]
    fn option_eq_form_not_reported_as_unknown_flag() {
        let rules = test_rules();
        let bash = test_bash_rules(&rules);
        // sort --output=/tmp/file should NOT be an unknown flag
        let (a, e) = args(&["sort", "--output=/tmp/sorted.txt", "input.txt"]);
        let result = evaluate_command(&a, &e, false, false, bash, "/tmp");
        match &result {
            EvalResult::Decided { reason, .. } => {
                assert!(
                    !reason.contains("unknown flag"),
                    "should not report --output=... as unknown flag: {reason}"
                );
            }
            EvalResult::CheckPaths { reason, .. } => {
                assert!(
                    !reason.contains("unknown flag"),
                    "should not report --output=... as unknown flag: {reason}"
                );
            }
        }
    }
}
