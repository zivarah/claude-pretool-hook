mod bash;
mod decision;
mod evaluate;
mod input;
mod path;
mod rules;

use std::{
    env,
    io::Read,
    path::{Path, PathBuf},
    process,
};

use anyhow::Context as _;
use clap::Parser;
use decision::{merge, Decision, EvalResult, PathCheck};
use input::{BashInput, HookInput, ToolInput};
use rules::Rules;
use serde::Serialize;

use crate::{
    decision::{
        Condition, ConditionalBranch, ConditionalDecisionNode, DecisionNode, StaticDecisionNode,
    },
    input::{EditInput, GlobInput, GrepInput, NotebookEditInput, ReadInput, WriteInput},
};

#[derive(Parser)]
#[command(name = "claude-pretool-hook")]
struct Cli {
    /// Path to the rules JSON file
    #[arg(long)]
    rules: String,
}

fn main() {
    let cli = Cli::parse();
    let rules_file = &cli.rules;

    let rules_content = match std::fs::read_to_string(rules_file) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Failed to read rules file '{rules_file}': {e}");
            process::exit(1);
        }
    };
    let rules: Rules = match serde_json::from_str(&rules_content) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Failed to parse rules JSON: {e}");
            process::exit(1);
        }
    };

    // Read hook input from stdin.
    let mut input = String::new();
    let response = if let Err(e) = std::io::stdin().read_to_string(&mut input) {
        create_response(Decision::Ask, &format!("failed to read stdin: {e}"))
    } else {
        match serde_json::from_str::<HookInput>(&input) {
            Ok(hook_input) => {
                let cwd = PathBuf::from(&hook_input.common.cwd);
                let project_dir = env::var("CLAUDE_PROJECT_DIR").ok();
                let compiled_fa = match path::CompiledFileAccess::compile(
                    &rules.file_access,
                    &cwd,
                    project_dir.as_deref(),
                ) {
                    Ok(c) => c,
                    Err(e) => {
                        eprintln!("Failed to compile file access patterns: {e}");
                        process::exit(1);
                    }
                };
                dispatch(&hook_input, &rules, &compiled_fa, &cwd)
                    .unwrap_or_else(|e| create_response(Decision::Ask, &format!("error: {e:#}")))
            }
            Err(e) => create_response(Decision::Ask, &format!("could not parse hook input: {e}")),
        }
    };

    print_response(response);
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HookSpecificOutput {
    pub hook_event_name: String,
    pub permission_decision: String,
    pub permission_decision_reason: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HookOutput {
    pub hook_specific_output: HookSpecificOutput,
}

fn print_response(output: HookOutput) {
    println!(
        "{}",
        serde_json::to_string(&output).expect("response serialization failed")
    );
}

fn create_response(decision: Decision, reason: &str) -> HookOutput {
    HookOutput {
        hook_specific_output: HookSpecificOutput {
            hook_event_name: "PreToolUse".to_owned(),
            permission_decision: decision.as_str().to_owned(),
            permission_decision_reason: reason.to_owned(),
        },
    }
}

/// Route a tool invocation to the appropriate handler based on the rule type.
fn dispatch(
    input: &HookInput,
    rules: &Rules,
    fa: &path::CompiledFileAccess,
    cwd: &Path,
) -> anyhow::Result<HookOutput> {
    let cwd_str = cwd.to_str().unwrap_or("/");
    let tool_input = &input.tool;
    Ok(match tool_input {
        ToolInput::Bash(input) => handle_bash(input, rules, fa, cwd)?,
        ToolInput::Edit(EditInput { file_path, .. }) => {
            handle_file_check(file_path, rules.tools.edit.as_ref(), fa, cwd)?
        }
        ToolInput::NotebookEdit(NotebookEditInput { file_path, .. }) => {
            handle_file_check(file_path, rules.tools.notebook_edit.as_ref(), fa, cwd)?
        }
        ToolInput::Read(ReadInput { file_path, .. }) => {
            handle_file_check(file_path, rules.tools.read.as_ref(), fa, cwd)?
        }
        ToolInput::Write(WriteInput { file_path, .. }) => {
            handle_file_check(file_path, rules.tools.write.as_ref(), fa, cwd)?
        }
        ToolInput::Glob(GlobInput { path, .. }) => {
            let path = path.as_deref().unwrap_or(cwd_str);
            handle_file_check(path, rules.tools.glob.as_ref(), fa, cwd)?
        }
        ToolInput::Grep(GrepInput { path, .. }) => {
            let path = path.as_deref().unwrap_or(cwd_str);
            handle_file_check(path, rules.tools.grep.as_ref(), fa, cwd)?
        }
        _ => {
            let decision = rules.tools.other.get(input.tool.name());
            match decision {
                Some(StaticDecisionNode(d)) => create_response(*d, "decision from rules"),
                None => create_response(Decision::Ask, "no rules for tool"),
            }
        }
    })
}

fn handle_file_check(
    file_path: &str,
    node: Option<&DecisionNode>,
    fa: &path::CompiledFileAccess,
    cwd: &Path,
) -> anyhow::Result<HookOutput> {
    let Some(node) = node else {
        return Ok(create_response(Decision::Ask, "no rules for tool"));
    };
    let decision = path::resolve_conditional(node, file_path, fa, cwd)
        .with_context(|| format!("resolving conditional for file '{file_path}'"))?;
    let reason = match node {
        DecisionNode::Static(_) => "decision from rules".to_owned(),
        DecisionNode::Conditional(cond) => {
            format!(
                "file '{file_path}' {} ({} check)",
                decision.description(),
                cond.condition.description()
            )
        }
    };
    Ok(create_response(decision, &reason))
}

fn handle_bash(
    bash_input: &BashInput,
    rules: &Rules,
    fa: &path::CompiledFileAccess,
    cwd: &Path,
) -> anyhow::Result<HookOutput> {
    let Some(bash_rules) = &rules.tools.bash else {
        return Ok(create_response(Decision::Ask, "no rules for bash tool"));
    };
    let cmd = &bash_input.command;
    if cmd.is_empty() {
        return Ok(create_response(Decision::Ask, "empty command"));
    }

    let (commands, redirects) =
        bash::parse(cmd).with_context(|| format!("parsing bash command: {cmd:?}"))?;

    let (results, all_plain, mut all_path_checks) = evaluate_commands(&commands, bash_rules, cwd);

    add_redirect_checks(&redirects, &mut all_path_checks);

    // If any plain decision is deny, return deny immediately.
    if all_plain.contains(&Decision::Deny) {
        let pairs = collect_eval_reason_pairs(&results);
        let reason = join_reasons(Decision::Deny, &pairs);
        return Ok(create_response(Decision::Deny, &reason));
    }

    // If there are path checks, resolve them and merge with plain decisions.
    if !all_path_checks.is_empty() {
        let path_results = resolve_path_checks(&all_path_checks, fa, cwd)?;
        return merge_with_path_results(&results, &all_plain, &path_results);
    }

    // Merge all plain decisions.
    if !all_plain.is_empty() {
        let merged = merge(&all_plain);
        let pairs = collect_eval_reason_pairs(&results);
        let reason = join_reasons(merged, &pairs);
        return Ok(create_response(merged, &reason));
    }

    Ok(create_response(Decision::Ask, "no commands to evaluate"))
}

/// Evaluate each parsed command (after stripping wrappers), collecting results
/// and separating path checks from plain decisions.
fn evaluate_commands(
    commands: &[bash::ExtractedCommand],
    bash_rules: &rules::BashRules,
    cwd: &Path,
) -> (Vec<EvalResult>, Vec<Decision>, Vec<PathCheck>) {
    let mut results = Vec::new();
    let mut all_plain = Vec::new();
    let mut all_path_checks = Vec::new();

    for command in commands {
        // Check globally allowed flags on original args before wrapper
        // stripping. Wrapper stripping can consume all args (e.g.,
        // `timeout --help` strips `timeout` then skipPositional eats
        // `--help`, leaving an empty command). The auto-allow check in
        // evaluate_command only sees the stripped args, so we catch it here.
        if command.args.len() == 2 && bash_rules.globally_allowed_flags.contains(&command.args[1]) {
            let result = EvalResult::Decided {
                decision: Decision::Allow,
                reason: format!(
                    "'{} {}' is always allowed",
                    command.args[0], command.args[1]
                ),
            };
            all_plain.push(Decision::Allow);
            results.push(result);
            continue;
        }

        let stripped = evaluate::strip_wrappers(command, bash_rules);
        let has_non_literal = stripped.expansion_flags.iter().any(|&e| e);
        let result = evaluate::evaluate_command(
            &stripped.args,
            &stripped.expansion_flags,
            has_non_literal,
            stripped.force_allow,
            bash_rules,
            cwd.to_str().unwrap_or("/"),
        );

        match &result {
            EvalResult::Decided { decision, .. } => {
                all_plain.push(*decision);
            }
            EvalResult::CheckPaths {
                base_decision,
                path_checks,
                ..
            } => {
                all_plain.push(*base_decision);
                all_path_checks.extend(path_checks.iter().map(|pc| PathCheck {
                    path: pc.path.clone(),
                    decision: pc.decision.clone(),
                    force: pc.force,
                }));
            }
        }

        results.push(result);
    }

    (results, all_plain, all_path_checks)
}

/// Add writable path checks for shell redirect targets (>, >>, etc.).
fn add_redirect_checks(redirects: &[bash::WriteRedirect], path_checks: &mut Vec<PathCheck>) {
    for redirect in redirects {
        path_checks.push(PathCheck {
            path: redirect.path.clone(),
            decision: DecisionNode::Conditional(Box::new(ConditionalDecisionNode {
                condition: Condition::Writable,
                then_decision: ConditionalBranch::Static(Decision::Allow),
                else_decision: ConditionalBranch::Nested(Box::new(ConditionalDecisionNode {
                    condition: Condition::Readable,
                    then_decision: ConditionalBranch::Static(Decision::Ask),
                    else_decision: ConditionalBranch::Static(Decision::Deny),
                })),
            })),
            force: false,
        });
    }
}

/// Resolve path checks against file-access glob patterns.
fn resolve_path_checks(
    path_checks: &[PathCheck],
    fa: &path::CompiledFileAccess,
    cwd: &Path,
) -> anyhow::Result<Vec<(Decision, bool, String)>> {
    let mut results = Vec::new();
    for pc in path_checks {
        let d = path::resolve_conditional(&pc.decision, &pc.path, fa, cwd)
            .with_context(|| format!("resolving path check for '{}'", pc.path))?;
        let reason = format!("path '{}' {}", pc.path, d.description());
        results.push((d, pc.force, reason));
    }
    Ok(results)
}

/// Merge plain decisions with resolved path results into a final response.
fn merge_with_path_results(
    results: &[EvalResult],
    all_plain: &[Decision],
    path_results: &[(Decision, bool, String)],
) -> anyhow::Result<HookOutput> {
    // Forced path results take priority. If they all agree, use that decision;
    // if they conflict, use Ask.
    let forced: Vec<_> = path_results.iter().filter(|(_, f, _)| *f).collect();
    if !forced.is_empty() {
        let first_d = forced[0].0;
        let all_reasons = forced
            .iter()
            .map(|(_, _, r)| r.as_str())
            .collect::<Vec<_>>()
            .join("; ");
        let (d, reason) = if forced.iter().all(|(d, _, _)| *d == first_d) {
            (first_d, all_reasons)
        } else {
            (
                Decision::Ask,
                format!("conflicting forced path decisions: {all_reasons}"),
            )
        };
        return Ok(create_response(d, &reason));
    }

    // Combine base decisions with path decisions.
    let all_decisions: Vec<Decision> = all_plain
        .iter()
        .chain(path_results.iter().map(|(d, _, _)| d))
        .copied()
        .collect();
    let merged = merge(&all_decisions);

    let mut pairs = collect_eval_reason_pairs(results);
    pairs.extend(path_results.iter().map(|(d, _, r)| (*d, r.as_str())));
    let reason = join_reasons(merged, &pairs);
    Ok(create_response(merged, &reason))
}

/// Collect (decision, reason) pairs from eval results.
fn collect_eval_reason_pairs(results: &[EvalResult]) -> Vec<(Decision, &str)> {
    results
        .iter()
        .filter_map(|r| match r {
            EvalResult::Decided { decision, reason }
            | EvalResult::CheckPaths {
                base_decision: decision,
                reason,
                ..
            } => {
                if reason.is_empty() {
                    None
                } else {
                    Some((*decision, reason.as_str()))
                }
            }
        })
        .collect()
}

/// Join reason fragments, filtering out Allow-decision fragments when the
/// final decision is not Allow. This reduces noise so the user only sees
/// what needs attention.
fn join_reasons(final_decision: Decision, pairs: &[(Decision, &str)]) -> String {
    if final_decision == Decision::Allow {
        return pairs.iter().map(|(_, r)| *r).collect::<Vec<_>>().join("; ");
    }
    let filtered: Vec<&str> = pairs
        .iter()
        .filter(|(d, _)| *d != Decision::Allow)
        .map(|(_, r)| *r)
        .collect();
    if filtered.is_empty() {
        // Shouldn't happen with a non-Allow final decision, but fall back
        // to showing everything rather than an empty string.
        pairs.iter().map(|(_, r)| *r).collect::<Vec<_>>().join("; ")
    } else {
        filtered.join("; ")
    }
}
