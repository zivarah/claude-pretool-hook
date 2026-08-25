use duct::cmd;

/// Path to the test rules fixture, relative to the crate root.
const TEST_RULES: &str = "tests/fixtures/test_rules.json";

/// A minimal rules fixture with `deferAskInAutoMode` enabled.
const DEFER_ASK_RULES: &str = "tests/fixtures/defer_ask_rules.json";

/// Run the hook binary with the given JSON on stdin, returning raw stdout.
fn run_hook_raw(input_json: &str) -> (i32, String) {
    run_hook_raw_with_mode("claude", input_json)
}

/// Run the hook in the selected mode with the default rules file.
fn run_hook_raw_with_mode(mode: &str, input_json: &str) -> (i32, String) {
    run_hook_raw_with_rules_and_mode(TEST_RULES, mode, input_json)
}

/// Run the hook binary against a specific rules file.
fn run_hook_raw_with_rules(rules: &str, input_json: &str) -> (i32, String) {
    run_hook_raw_with_rules_and_mode(rules, "claude", input_json)
}

/// Run the hook in the selected mode against a specific rules file.
fn run_hook_raw_with_rules_and_mode(rules: &str, mode: &str, input_json: &str) -> (i32, String) {
    let bin = env!("CARGO_BIN_EXE_claude-pretool-hook");
    let output = cmd!(bin, "--mode", mode, "--rules", rules)
        .stdin_bytes(input_json.as_bytes())
        .stdout_capture()
        .stderr_capture()
        .unchecked()
        .run()
        .expect("failed to run hook binary");

    let code = output.status.code().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    (code, stdout)
}

/// Run the hook and normalize either output format to a canonical
/// `(decision, reason)`
fn run_hook(input_json: &str) -> (i32, String, String) {
    let (code, stdout) = run_hook_raw(input_json);
    if stdout.is_empty() {
        return (code, String::new(), String::new());
    }

    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap();

    if let Some(hook_output) = parsed.get("hookSpecificOutput") {
        let decision = hook_output["permissionDecision"]
            .as_str()
            .unwrap()
            .to_string();
        let reason = hook_output["permissionDecisionReason"]
            .as_str()
            .unwrap()
            .to_string();
        return (code, decision, reason);
    }

    // Codex format.
    let decision = match parsed.get("decision").and_then(|v| v.as_str()) {
        Some("approve") => "allow",
        Some("block") => "deny",
        _ => "ask",
    }
    .to_string();
    let reason = parsed
        .get("reason")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    (code, decision, reason)
}

/// Shorthand for a Bash tool invocation.
fn bash_input(command: &str) -> String {
    serde_json::json!({
        "tool_name": "Bash",
        "tool_input": { "command": command }
    })
    .to_string()
}

/// Shorthand for a Bash tool invocation with a specific cwd.
fn bash_input_with_cwd(command: &str, cwd: &str) -> String {
    serde_json::json!({
        "tool_name": "Bash",
        "tool_input": { "command": command },
        "cwd": cwd
    })
    .to_string()
}

/// Shorthand for a Codex-style Bash invocation with extra Codex fields.
fn codex_bash_input(command: &str) -> String {
    serde_json::json!({
        "tool_name": "Bash",
        "tool_input": { "command": command },
        "hook_event_name": "PreToolUse",
        "session_id": "s-1",
        "model": "gpt-5-codex",
        "turn_id": "t-1"
    })
    .to_string()
}

/// Shorthand for an `apply_patch` invocation carrying a patch envelope.
fn apply_patch_input(patch: &str) -> String {
    serde_json::json!({
        "tool_name": "apply_patch",
        "tool_input": { "input": patch }
    })
    .to_string()
}

/// Shorthand for a Codex `apply_patch` invocation.
fn codex_apply_patch_input(patch: &str) -> String {
    serde_json::json!({
        "tool_name": "apply_patch",
        "tool_input": { "input": patch },
        "hook_event_name": "PreToolUse"
    })
    .to_string()
}

/// Shorthand for a Read tool invocation.
fn read_input(file_path: &str) -> String {
    serde_json::json!({
        "tool_name": "Read",
        "tool_input": { "file_path": file_path }
    })
    .to_string()
}

/// Shorthand for a Write tool invocation.
fn write_input(file_path: &str) -> String {
    serde_json::json!({
        "tool_name": "Write",
        "tool_input": { "file_path": file_path, "content": "" }
    })
    .to_string()
}

/// Shorthand for an Edit tool invocation.
fn edit_input(file_path: &str) -> String {
    serde_json::json!({
        "tool_name": "Edit",
        "tool_input": { "file_path": file_path, "old_string": "", "new_string": "" }
    })
    .to_string()
}

/// Shorthand for a NotebookEdit tool invocation.
fn notebook_edit_input(file_path: &str) -> String {
    serde_json::json!({
        "tool_name": "NotebookEdit",
        "tool_input": { "file_path": file_path }
    })
    .to_string()
}

/// Shorthand for a Glob tool invocation with an optional path.
fn glob_input_with_path(pattern: &str, path: Option<&str>) -> String {
    let mut input = serde_json::json!({ "pattern": pattern });
    if let Some(p) = path {
        input["path"] = serde_json::json!(p);
    }
    serde_json::json!({
        "tool_name": "Glob",
        "tool_input": input
    })
    .to_string()
}

/// Shorthand for a Grep tool invocation with an optional path.
fn grep_input_with_path(pattern: &str, path: Option<&str>) -> String {
    let mut input = serde_json::json!({ "pattern": pattern });
    if let Some(p) = path {
        input["path"] = serde_json::json!(p);
    }
    serde_json::json!({
        "tool_name": "Grep",
        "tool_input": input
    })
    .to_string()
}

/// Shorthand for a WebFetch tool invocation.
fn web_fetch_input(url: &str) -> String {
    serde_json::json!({
        "tool_name": "WebFetch",
        "tool_input": { "url": url, "prompt": "summarize" }
    })
    .to_string()
}

/// Shorthand for an Agent tool invocation.
fn agent_input(prompt: &str) -> String {
    serde_json::json!({
        "tool_name": "Agent",
        "tool_input": { "prompt": prompt }
    })
    .to_string()
}

/// Shorthand for an AskUserQuestion tool invocation.
fn ask_user_input() -> String {
    serde_json::json!({
        "tool_name": "AskUserQuestion",
        "tool_input": {
            "questions": [{
                "question": "Proceed?",
                "header": "Confirm",
                "options": [{ "label": "Yes" }]
            }]
        }
    })
    .to_string()
}

/// Shorthand for a tool with no structured input (used for unknown/passthrough tools).
fn tool_input(tool_name: &str) -> String {
    serde_json::json!({
        "tool_name": tool_name,
        "tool_input": {}
    })
    .to_string()
}

fn assert_decision(input: &str, expected: &str) {
    let (code, decision, reason) = run_hook(input);
    assert_eq!(code, 0, "expected exit 0, got {code}; reason: {reason}");
    assert_eq!(
        decision, expected,
        "expected '{expected}', got '{decision}'; reason: {reason}"
    );
}

// =============================================================================
// Bash tool — simple decisions
// =============================================================================

#[test]
fn bash_simple_allow() {
    // ls is a simple "allow" command
    assert_decision(&bash_input("ls"), "allow");
}

#[test]
fn bash_simple_deny() {
    // dd is a simple "deny" command
    assert_decision(&bash_input("dd"), "deny");
}

#[test]
fn bash_simple_ask() {
    // sh has decision "ask"
    assert_decision(&bash_input("sh"), "ask");
}

#[test]
fn bash_unknown_command() {
    assert_decision(&bash_input("totally-unknown"), "ask");
}

#[test]
fn bash_empty_command() {
    assert_decision(&bash_input(""), "ask");
}

// =============================================================================
// Bash tool — --help / --version
// =============================================================================

#[test]
fn bash_help_overrides_deny() {
    // dd is deny, but --help always overrides
    assert_decision(&bash_input("dd --help"), "allow");
}

#[test]
fn bash_version_overrides_deny() {
    // dd is deny, but --version always overrides
    assert_decision(&bash_input("dd --version"), "allow");
}

#[test]
fn bash_wrapper_help_allowed() {
    // timeout --help: wrapper strips `timeout`, --help is the only remaining
    // arg — should be auto-allowed, not "unknown command"
    assert_decision(&bash_input("timeout --help"), "allow");
}

#[test]
fn bash_wrapper_command_help_allowed() {
    // command --help: wrapper strips `command`, --help remains
    assert_decision(&bash_input("command --help"), "allow");
}

#[test]
fn bash_wrapper_version_allowed() {
    assert_decision(&bash_input("timeout --version"), "allow");
}

#[test]
fn bash_wrapper_help_in_pipe_allowed() {
    // timeout --help 2>&1 | head -50: both commands should be allowed
    assert_decision(&bash_input("timeout --help 2>&1 | head -50"), "allow");
}

// =============================================================================
// Bash tool — subcmds
// =============================================================================

#[test]
fn bash_subcmd_allow() {
    // cargo test → subcmd "test" is allow
    assert_decision(&bash_input("cargo test"), "allow");
}

#[test]
fn bash_subcmd_deny() {
    // git push → subcmd "push" is deny
    assert_decision(&bash_input("git push"), "deny");
}

#[test]
fn bash_subcmd_wildcard() {
    // npm audit → matches "*" wildcard (ask)
    assert_decision(&bash_input("npm audit"), "ask");
}

#[test]
fn bash_nested_subcmd_allow() {
    // nix flake check → nested subcmd "check" is allow
    assert_decision(&bash_input("nix flake check"), "allow");
}

#[test]
fn bash_nested_subcmd_deny() {
    // git clean → subcmd "clean" is deny
    assert_decision(&bash_input("git clean"), "deny");
}

// =============================================================================
// Bash tool — pre-subcmd options with conditional values
// =============================================================================

#[test]
fn bash_pre_subcmd_option_conditional_writable() {
    // git -C /tmp/foo status → -C value /tmp/foo is writable → allow,
    // merged with subcmd "status" allow → allow
    assert_decision(&bash_input("git -C /tmp/foo status"), "allow");
}

#[test]
fn bash_pre_subcmd_option_conditional_not_writable() {
    // git -C /etc/foo log → -C value /etc/foo is not writable but readable
    // → ask, merged with subcmd "log" allow → ask
    assert_decision(&bash_input("git -C /etc/foo log"), "ask");
}

// =============================================================================
// Bash tool — flags
// =============================================================================

#[test]
fn bash_flag_allow() {
    // make -j → flag "-j" is allow
    assert_decision(&bash_input("make -j"), "allow");
}

#[test]
fn bash_flag_deny() {
    // rm -r → flag "-r" is deny
    assert_decision(&bash_input("rm -r"), "deny");
}

#[test]
fn bash_flag_alias() {
    // make -n → alias for --dry-run, which is allow
    assert_decision(&bash_input("make -n"), "allow");
}

#[test]
fn bash_flag_wildcard() {
    // make --unknown-flag → matches "*" wildcard (allow)
    assert_decision(&bash_input("make --unknown-flag"), "allow");
}

#[test]
fn bash_flag_not_listed_no_wildcard() {
    // file has flags ({-C: ask}) but no "*" wildcard. An unknown flag must
    // produce ask, overriding the node's base "allow".
    assert_decision(&bash_input("file --unknown-flag /tmp/foo"), "ask");
}

#[test]
fn bash_flag_deny_wins() {
    // rm has decision allow, -r is deny → deny wins
    assert_decision(&bash_input("rm -r /tmp/file.txt"), "deny");
}

// =============================================================================
// Bash tool — options with values
// =============================================================================

#[test]
fn bash_option_value_allow() {
    // curl --request GET → value "GET" is allow
    assert_decision(&bash_input("curl --request GET"), "allow");
}

#[test]
fn bash_option_value_deny() {
    // curl --request DELETE → value "DELETE" is deny
    assert_decision(&bash_input("curl --request DELETE"), "deny");
}

#[test]
fn bash_option_value_wildcard() {
    // curl --request POST → matches "*" wildcard (ask)
    assert_decision(&bash_input("curl --request POST"), "ask");
}

#[test]
fn bash_option_alias() {
    // curl -X GET → -X is alias for --request, value "GET" is allow
    assert_decision(&bash_input("curl -X GET"), "allow");
}

#[test]
fn bash_option_no_values_dict() {
    // git commit -m "msg" → -m has no values dict, just decision allow + allowExpansions
    assert_decision(&bash_input("git commit -m 'initial commit'"), "allow");
}

#[test]
fn bash_option_not_reported_as_unknown_flag() {
    // sed has both flags and options; --expression is in options, not flags.
    // The flags loop must skip it rather than reporting it as unknown.
    let input = write_tmp_file("");
    let cmd = format!("sed --expression 's/foo/bar/' {}", input.path().display());
    assert_decision(&bash_input(&cmd), "allow");
}

#[test]
fn bash_option_alias_not_reported_as_unknown_flag() {
    // sed -e is an alias for --expression (in options, not flags).
    let input = write_tmp_file("");
    let cmd = format!("sed -e 's/foo/bar/' {}", input.path().display());
    assert_decision(&bash_input(&cmd), "allow");
}

// =============================================================================
// Bash tool — positional
// =============================================================================

#[test]
fn bash_positional_count_1_allow() {
    // cd /tmp/foo → count "1" has conditional (readable → allow)
    assert_decision(&bash_input("cd /tmp/foo"), "allow");
}

#[test]
fn bash_positional_count_2_ask() {
    // cd /tmp/a /tmp/b → 2 positionals, no "2" key, no wildcard → ask
    assert_decision(&bash_input("cd /tmp/a /tmp/b"), "ask");
}

#[test]
fn bash_positional_wildcard_count() {
    // awk a b c → "*" wildcard positional is allow
    assert_decision(&bash_input("awk a b c"), "allow");
}

// =============================================================================
// Bash tool — single positional with conditional decision
// =============================================================================

#[test]
fn bash_positional_single_conditional_readable() {
    // cd /tmp/foo → readable (not denied) → allow
    assert_decision(&bash_input("cd /tmp/foo"), "allow");
}

#[test]
fn bash_positional_single_conditional_denied() {
    // cd /tmp/app.secret → basename matches deny pattern → not readable → deny
    assert_decision(&bash_input("cd /tmp/app.secret"), "deny");
}

// =============================================================================
// Bash tool — positional with path conditionals
// =============================================================================

#[test]
fn bash_transfer_writable_paths() {
    // cp /tmp/src.txt /tmp/dst.txt → count "2" array [readable, writable], both in /tmp/ → allow
    assert_decision(&bash_input("cp /tmp/src.txt /tmp/dst.txt"), "allow");
}

#[test]
fn bash_transfer_non_writable_dest() {
    // cp: source readable, dest not writable → ask
    assert_decision(&bash_input("cp /tmp/src.txt /etc/dst.txt"), "ask");
}

#[test]
fn bash_transfer_denied_path() {
    // cp: source matches deny pattern → readable fails → deny (stricter wins)
    assert_decision(&bash_input("cp /tmp/app.secret /tmp/dst.txt"), "deny");
}

// =============================================================================
// Bash tool — conditional option values
// =============================================================================

#[test]
fn bash_option_value_conditional_writable() {
    // curl --output /tmp/out.txt → value wildcard has {if:writable → allow}
    assert_decision(&bash_input("curl --output /tmp/out.txt"), "allow");
}

#[test]
fn bash_option_value_conditional_not_writable() {
    // curl --output /etc/out.txt → not writable → deny
    assert_decision(&bash_input("curl --output /etc/out.txt"), "deny");
}

// =============================================================================
// Bash tool — wrappers
// =============================================================================

#[test]
fn bash_wrapper_strips_to_inner() {
    // timeout --foreground 5 ls → strips timeout, consumes --foreground, skips "5", evaluates "ls" → allow
    assert_decision(&bash_input("timeout --foreground 5 ls"), "allow");
}

#[test]
fn bash_wrapper_inner_deny() {
    // timeout 5 dd → strips timeout, skips "5", evaluates "dd" → deny
    assert_decision(&bash_input("timeout 5 dd"), "deny");
}

#[test]
fn bash_force_wrapper() {
    // command -v dd → -v has force=true → force_allow overrides dd's deny
    assert_decision(&bash_input("command -v dd"), "allow");
}

#[test]
fn bash_force_wrapper_alias() {
    // command -V dd → -V also has force=true
    assert_decision(&bash_input("command -V dd"), "allow");
}

#[test]
fn bash_chained_wrappers() {
    // timeout 5 timeout 10 ls → both wrappers stripped, evaluates "ls" → allow
    assert_decision(&bash_input("timeout 5 timeout 10 ls"), "allow");
}

// =============================================================================
// Bash tool — expansions
// =============================================================================

#[test]
fn bash_expansion_uncovered() {
    // ls $MY_VAR → ls has no allowExpansions → ask
    assert_decision(&bash_input("ls $MY_VAR"), "ask");
}

#[test]
fn bash_expansion_node_allows() {
    // env $MY_VAR → env has allowExpansions: true → allow
    assert_decision(&bash_input("env $MY_VAR"), "allow");
}

#[test]
fn bash_expansion_option_allows() {
    // git commit -m $MSG → -m has allowExpansions: true → allow
    assert_decision(&bash_input("git commit -m $MSG"), "allow");
}

// =============================================================================
// Bash tool — pipes and chains
// =============================================================================

#[test]
fn bash_pipe_both_allowed() {
    // ls foo | cat → both allow
    assert_decision(&bash_input("ls foo | cat"), "allow");
}

#[test]
fn bash_pipe_one_denied() {
    // ls foo | dd → dd is deny
    assert_decision(&bash_input("ls foo | dd"), "deny");
}

#[test]
fn bash_chain_and() {
    // ls foo && cat bar → both allow
    assert_decision(&bash_input("ls foo && cat bar"), "allow");
}

#[test]
fn bash_chain_semicolon_deny() {
    // ls foo; dd → dd is deny
    assert_decision(&bash_input("ls foo; dd"), "deny");
}

// =============================================================================
// Bash tool — reason filtering (no "is approved" noise in ask/deny)
// =============================================================================

#[test]
fn bash_pipe_deny_reason_omits_approved() {
    // ls foo | dd → dd is deny, ls is allow.
    // The reason should mention dd being denied but NOT "ls" being approved.
    let (_, decision, reason) = run_hook(&bash_input("ls foo | dd"));
    assert_eq!(decision, "deny");
    assert!(
        !reason.contains("is approved"),
        "deny reason should not contain 'is approved' fragments: {reason}"
    );
    assert!(
        reason.contains("is denied"),
        "deny reason should mention the denied command: {reason}"
    );
}

#[test]
fn bash_pipe_ask_reason_omits_approved() {
    // cat foo | totally-unknown-cmd → unknown is ask, cat is allow.
    let (_, decision, reason) = run_hook(&bash_input("cat foo | totally-unknown-cmd"));
    assert_eq!(decision, "ask");
    assert!(
        !reason.contains("is approved"),
        "ask reason should not contain 'is approved' fragments: {reason}"
    );
}

#[test]
fn bash_allow_reason_keeps_approved() {
    // ls foo | cat → both allow. Reason should still contain "is approved".
    let (_, decision, reason) = run_hook(&bash_input("ls foo | cat"));
    assert_eq!(decision, "allow");
    assert!(
        reason.contains("is approved"),
        "allow reason should keep 'is approved' fragments: {reason}"
    );
}

// =============================================================================
// Bash tool — redirects
// =============================================================================

#[test]
fn bash_redirect_writable() {
    // ls foo > /tmp/out.txt → writable path → allow
    assert_decision(&bash_input("ls foo > /tmp/out.txt"), "allow");
}

#[test]
fn bash_redirect_not_writable() {
    assert_decision(&bash_input("ls foo > /etc/out.txt"), "ask");
}

#[test]
fn bash_redirect_denied_pattern() {
    assert_decision(&bash_input("ls foo > /tmp/app.secret"), "deny");
}

// =============================================================================
// Bash tool — command substitution
// =============================================================================

#[test]
fn bash_command_substitution_inner_deny() {
    // Inner command is denied, even if outer is allowed
    assert_decision(&bash_input("curl --request $(dd)"), "deny");
}

// =============================================================================
// Read tool
// =============================================================================

#[test]
fn read_allowed_path() {
    assert_decision(&read_input("/home/user/readme.md"), "allow");
}

#[test]
fn read_denied_path() {
    assert_decision(&read_input("/home/user/.secret"), "deny");
}

#[test]
fn read_denied_key_file() {
    assert_decision(&read_input("/etc/api.key"), "deny");
}

#[test]
fn read_empty_path() {
    assert_decision(&read_input(""), "allow");
}

// =============================================================================
// Write / Edit / NotebookEdit tools
// =============================================================================

#[test]
fn write_allowed_prefix() {
    assert_decision(&write_input("/tmp/output.txt"), "allow");
}

#[test]
fn write_workspace_prefix() {
    assert_decision(&write_input("/workspace/src/main.rs"), "allow");
}

#[test]
fn write_outside_prefix() {
    assert_decision(&write_input("/etc/config.txt"), "ask");
}

#[test]
fn write_denied_pattern() {
    assert_decision(&write_input("/tmp/app.secret"), "deny");
}

#[test]
fn edit_allowed() {
    assert_decision(&edit_input("/tmp/file.rs"), "allow");
}

#[test]
fn notebook_edit_allowed() {
    assert_decision(&notebook_edit_input("/workspace/nb.ipynb"), "allow");
}

#[test]
fn write_empty_path() {
    assert_decision(&write_input(""), "ask");
}

// =============================================================================
// Auto-allow tools
// One representative per input-parsing shape: passthrough (LSP), glob/grep
// pattern, web fetch (url+prompt), agent (prompt), ask-user (complex).
// =============================================================================

#[test]
fn auto_allow_passthrough_tool() {
    // Tools with no structured input and a static allow rule.
    assert_decision(&tool_input("LSP"), "allow");
}

#[test]
fn glob_no_path_uses_cwd() {
    // No path → defaults to cwd, which is readable → allow
    assert_decision(&glob_input_with_path("**/*.rs", None), "allow");
}

#[test]
fn glob_readable_path() {
    assert_decision(
        &glob_input_with_path("**/*.rs", Some("/tmp/project")),
        "allow",
    );
}

#[test]
fn glob_denied_path() {
    // Path matching a deny pattern → not readable → deny
    assert_decision(
        &glob_input_with_path("**/*.rs", Some("/tmp/app.secret")),
        "deny",
    );
}

#[test]
fn grep_no_path_uses_cwd() {
    assert_decision(&grep_input_with_path("TODO", None), "allow");
}

#[test]
fn grep_readable_path() {
    assert_decision(&grep_input_with_path("TODO", Some("/tmp/project")), "allow");
}

#[test]
fn grep_denied_path() {
    assert_decision(
        &grep_input_with_path("password", Some("/tmp/app.secret")),
        "deny",
    );
}

#[test]
fn auto_allow_web_fetch() {
    assert_decision(&web_fetch_input("https://example.com"), "allow");
}

#[test]
fn auto_allow_agent() {
    assert_decision(&agent_input("explore the codebase"), "allow");
}

#[test]
fn auto_allow_ask_user() {
    assert_decision(&ask_user_input(), "allow");
}

// =============================================================================
// Unknown tool
// =============================================================================

#[test]
fn unknown_tool_ask() {
    assert_decision(&tool_input("SomeBrandNewTool"), "ask");
}

// =============================================================================
// Bash tool — find with dangerous flags
// =============================================================================

#[test]
fn bash_find_delete_denied() {
    assert_decision(&bash_input("find /tmp -delete"), "deny");
}

#[test]
fn bash_find_exec_denied() {
    assert_decision(&bash_input("find /tmp -exec rm {} \\;"), "deny");
}

#[test]
fn bash_find_execdir_denied() {
    assert_decision(&bash_input("find /tmp -execdir cat {} \\;"), "deny");
}

#[test]
fn bash_find_normal_flags_allowed() {
    assert_decision(&bash_input("find /tmp -name '*.txt' -type f"), "allow");
}

#[test]
fn bash_find_fprint_writable() {
    assert_decision(&bash_input("find /tmp -fprint /tmp/out.txt"), "allow");
}

#[test]
fn bash_find_fprint_not_writable() {
    // /etc/out.txt is readable but not writable → ask
    assert_decision(&bash_input("find /tmp -fprint /etc/out.txt"), "ask");
}

// =============================================================================
// Bash tool — sort with -o
// =============================================================================

#[test]
fn bash_sort_no_output_allowed() {
    assert_decision(&bash_input("sort /tmp/file.txt"), "allow");
}

#[test]
fn bash_sort_output_writable() {
    assert_decision(&bash_input("sort -o /tmp/out.txt /tmp/in.txt"), "allow");
}

#[test]
fn bash_sort_output_not_writable() {
    // /etc/out.txt is readable but not writable → ask
    assert_decision(&bash_input("sort -o /etc/out.txt /tmp/in.txt"), "ask");
}

// =============================================================================
// Bash tool — shfmt with -w flag positional overlay
// =============================================================================

#[test]
fn bash_shfmt_read_only_allowed() {
    assert_decision(&bash_input("shfmt /tmp/script.sh"), "allow");
}

#[test]
fn bash_shfmt_read_only_secret() {
    // /tmp/app.secret is denied by read patterns → deny
    assert_decision(&bash_input("shfmt /tmp/app.secret"), "deny");
}

#[test]
fn bash_shfmt_write_writable() {
    // /tmp/ is both readable and writable → allow
    assert_decision(&bash_input("shfmt -w /tmp/script.sh"), "allow");
}

#[test]
fn bash_shfmt_write_not_writable() {
    // /etc/script.sh is readable but not writable → ask (from -w overlay)
    assert_decision(&bash_input("shfmt -w /etc/script.sh"), "ask");
}

// TODO: shfmt -w with no positional (reads stdin) still passes because
// there are no positionals to check. Acceptable since writing stdin back
// to nothing is a no-op.

// =============================================================================
// Bash tool — awk -f (path gating + checkFile unreadable fallback)
// =============================================================================

/// Writes `contents` to a `NamedTempFile` and returns it. The file is
/// auto-deleted when the returned handle is dropped, so tests just need
/// to keep the binding alive for the duration of the assertion.
fn write_tmp_file(contents: &str) -> tempfile::NamedTempFile {
    use std::io::Write;
    let mut file = tempfile::NamedTempFile::new().expect("create tmp file");
    file.write_all(contents.as_bytes()).expect("write tmp file");
    file.flush().expect("flush tmp file");
    file
}

#[test]
fn bash_awk_no_file_allowed() {
    assert_decision(&bash_input("awk '{print $1}' /tmp/data.txt"), "allow");
}

#[test]
fn bash_awk_f_blocked_path_denies() {
    // /tmp/app.secret is denied by read patterns → checkFile reports
    // unreadable and the default `onUnreadable: deny` applies. The file
    // doesn't need to exist; the read-globs gate runs before any I/O.
    assert_decision(&bash_input("awk -f /tmp/app.secret /tmp/data.txt"), "deny");
}

#[test]
fn bash_awk_f_missing_file_denies() {
    // path passes the read-globs gate but doesn't exist on disk →
    // onUnreadable defaults to deny.
    assert_decision(
        &bash_input("awk -f /tmp/cph-it-nonexistent-xyz.awk /tmp/data.txt"),
        "deny",
    );
}

// =============================================================================
// Bash tool — sed/awk danger-pattern detection
// =============================================================================

// --- sed -e (option value patterns) ---

#[test]
fn bash_sed_e_safe_substitution_allow() {
    let input = write_tmp_file("");
    let cmd = format!("sed -e 's/foo/bar/' {}", input.path().display());
    assert_decision(&bash_input(&cmd), "allow");
}

#[test]
fn bash_sed_e_exec_command_asks() {
    // The `e` sed command executes the pattern space as a shell command.
    let input = write_tmp_file("");
    let cmd = format!("sed -e 'e cat /etc/passwd' {}", input.path().display());
    assert_decision(&bash_input(&cmd), "ask");
}

#[test]
fn bash_sed_e_substitution_e_flag_asks() {
    // s///e flag executes the substitution result as a shell command.
    let input = write_tmp_file("");
    let cmd = format!("sed -e 's/foo/bar/e' {}", input.path().display());
    assert_decision(&bash_input(&cmd), "ask");
}

#[test]
fn bash_sed_e_write_command_asks() {
    // The `w` sed command writes pattern space to a file (other than stdout).
    let input = write_tmp_file("");
    let cmd = format!("sed -e '1w /etc/passwd' {}", input.path().display());
    assert_decision(&bash_input(&cmd), "ask");
}

// --- sed positional script (no -e) ---

#[test]
fn bash_sed_positional_safe_script_allow() {
    let input = write_tmp_file("");
    let cmd = format!("sed 's/foo/bar/' {}", input.path().display());
    assert_decision(&bash_input(&cmd), "allow");
}

#[test]
fn bash_sed_positional_exec_command_asks() {
    let input = write_tmp_file("");
    let cmd = format!("sed '1e cat /etc/passwd' {}", input.path().display());
    assert_decision(&bash_input(&cmd), "ask");
}

#[test]
fn bash_sed_e_with_unreadable_file_path_denies() {
    // overridePositional skips the parent's positional rule, but the option's
    // own positional overlay (ifReadable) still runs against the file arg.
    // A path blocked by the read globs → ifReadable → deny.
    assert_decision(&bash_input("sed -e 's/foo/bar/' /tmp/app.secret"), "deny");
}

#[test]
fn bash_sed_e_with_file_path_matching_danger_regex_allow() {
    // When -e carries the script, the lone positional is the input file.
    // The file path shouldn't be screened against script-danger patterns
    // since the script came from -e. Uses a deep nested path that the
    // s/// danger regex would match if it were applied to the file slot.
    let dir = tempfile::TempDir::new().expect("create tmp dir");
    let nested = dir.path().join("things/foo/bar");
    std::fs::create_dir_all(&nested).expect("create dirs");
    let file = nested.join("home.txt");
    std::fs::write(&file, "").expect("write file");
    let cmd = format!("sed -e 's/foo/bar/' {}", file.display());
    assert_decision(&bash_input(&cmd), "allow");
}

#[test]
fn bash_sed_positional_file_path_not_screened_as_script() {
    // Regression: the danger regex `s/[^/]+/[^/]+/[a-z]*[ew]` can spuriously
    // match deep file paths. `/<tmp>/things/foo/bar/home.txt` contains the
    // substring `s/foo/bar/home` (s + two `/`-separated segments + a third
    // segment ending in "e"), which matches the substitution-with-e/w-flag
    // pattern. The script (positional 1, "44,72p") is safe; the file-path
    // positional shouldn't be screened against script-danger regexes.
    let dir = tempfile::TempDir::new().expect("create tmp dir");
    let nested = dir.path().join("things/foo/bar");
    std::fs::create_dir_all(&nested).expect("create dirs");
    let file = nested.join("home.txt");
    std::fs::write(&file, "").expect("write file");
    let cmd = format!("sed -n 44,72p {}", file.display());
    assert_decision(&bash_input(&cmd), "allow");
}

// --- sed -f (file-contents patterns) ---

#[test]
fn bash_sed_f_safe_script_file_allow() {
    let script = write_tmp_file("s/foo/bar/");
    let input = write_tmp_file("");
    let cmd = format!(
        "sed -f {} {}",
        script.path().display(),
        input.path().display()
    );
    assert_decision(&bash_input(&cmd), "allow");
}

#[test]
fn bash_sed_f_evil_script_file_asks() {
    let script = write_tmp_file("1e rm -rf /\ns/a/b/");
    let input = write_tmp_file("");
    let cmd = format!(
        "sed -f {} {}",
        script.path().display(),
        input.path().display()
    );
    assert_decision(&bash_input(&cmd), "ask");
}

// --- awk -f (file-contents patterns) ---

#[test]
fn bash_awk_f_safe_script_file_allow() {
    let tmp = write_tmp_file("{print $1}");
    let cmd = format!("awk -f {} /tmp/data.txt", tmp.path().display());
    assert_decision(&bash_input(&cmd), "allow");
}

#[test]
fn bash_awk_f_system_call_asks() {
    let tmp = write_tmp_file("BEGIN{system(\"id\")}");
    let cmd = format!("awk -f {} /tmp/data.txt", tmp.path().display());
    assert_decision(&bash_input(&cmd), "ask");
}

#[test]
fn bash_awk_f_print_redirect_asks() {
    let tmp = write_tmp_file("{print $1 > \"/tmp/out\"}");
    let cmd = format!("awk -f {} /tmp/data.txt", tmp.path().display());
    assert_decision(&bash_input(&cmd), "ask");
}

// --- awk positional script ---

#[test]
fn bash_awk_positional_safe_script_allow() {
    assert_decision(&bash_input("awk '{print $1}' /tmp/data.txt"), "allow");
}

#[test]
fn bash_awk_positional_system_call_asks() {
    assert_decision(
        &bash_input("awk 'BEGIN{system(\"rm -rf /\")}' /tmp/data.txt"),
        "ask",
    );
}

#[test]
fn bash_awk_positional_print_redirect_asks() {
    assert_decision(
        &bash_input("awk '{print > \"/tmp/x\"}' /tmp/data.txt"),
        "ask",
    );
}

// =============================================================================
// Bash tool — jq file-reading options
// =============================================================================

#[test]
fn bash_jq_no_file_allowed() {
    assert_decision(&bash_input("jq '.foo' /tmp/data.json"), "allow");
}

#[test]
fn bash_jq_from_file_readable() {
    assert_decision(&bash_input("jq -f /tmp/filter.jq /tmp/data.json"), "allow");
}

#[test]
fn bash_jq_from_file_not_readable() {
    // /tmp/app.secret is denied by read patterns → deny
    assert_decision(&bash_input("jq -f /tmp/app.secret"), "deny");
}

// =============================================================================
// Bash tool — cp/mv with -t (target directory)
// =============================================================================

#[test]
fn bash_cp_target_dir_writable() {
    // -t /tmp/dest is writable → allow from option check. But src.txt is
    // a positional matching "*" = "ask", so merged result is ask.
    // TODO: when -t is used, remaining positionals are sources (not dest).
    // The existing positional "*" = "ask" is overly strict here. A flag
    // positional overlay could relax this.
    assert_decision(&bash_input("cp -t /tmp/dest src.txt"), "ask");
}

#[test]
fn bash_cp_target_dir_not_writable() {
    // /etc/dest is readable but not writable → ask (from option check)
    assert_decision(&bash_input("cp -t /etc/dest src.txt"), "ask");
}

#[test]
fn bash_cp_target_dir_no_extra_positionals() {
    // With no positional args beyond -t's value, only the option is checked
    assert_decision(&bash_input("cp -t /tmp/dest"), "allow");
}

#[test]
fn bash_mv_target_dir_writable() {
    // Same as cp: positional "*" = "ask" makes this ask despite writable -t
    assert_decision(&bash_input("mv -t /tmp/dest src.txt"), "ask");
}

#[test]
fn bash_mv_target_dir_not_writable() {
    // /etc/dest is readable but not writable → ask (from option check)
    assert_decision(&bash_input("mv -t /etc/dest src.txt"), "ask");
}

#[test]
fn bash_mv_target_dir_no_extra_positionals() {
    assert_decision(&bash_input("mv -t /tmp/dest"), "allow");
}

// =============================================================================
// Bash tool — git config subcmds, git -c asks, git --output
// =============================================================================

#[test]
fn bash_git_config_get_allowed() {
    assert_decision(&bash_input("git config get user.name"), "allow");
}

#[test]
fn bash_git_config_list_allowed() {
    assert_decision(&bash_input("git config list"), "allow");
}

#[test]
fn bash_git_config_bare_asks() {
    // git config without a recognized subcmd asks for approval
    assert_decision(&bash_input("git config user.name foo"), "ask");
}

#[test]
fn bash_git_dash_c_asks() {
    // git -c can set dangerous config (core.hooksPath, include.path)
    assert_decision(&bash_input("git -c core.editor=vim status"), "ask");
}

#[test]
fn bash_git_show_output_writable() {
    assert_decision(&bash_input("git show --output /tmp/out.patch"), "allow");
}

#[test]
fn bash_git_show_output_not_writable() {
    // /etc/out.patch is readable but not writable → ask
    assert_decision(&bash_input("git show --output /etc/out.patch"), "ask");
}

#[test]
fn bash_git_diff_output_writable() {
    assert_decision(&bash_input("git diff --output /tmp/out.patch"), "allow");
}

#[test]
fn bash_git_log_output_writable() {
    assert_decision(&bash_input("git log --output /tmp/out.log"), "allow");
}

// =============================================================================
// Bash tool — --option=value (equals form) splitting
// =============================================================================

#[test]
fn bash_git_show_output_eq_writable() {
    // --output=/tmp/out.patch should be split into --output + /tmp/out.patch
    assert_decision(&bash_input("git show --output=/tmp/out.patch"), "allow");
}

#[test]
fn bash_git_show_output_eq_not_writable() {
    assert_decision(&bash_input("git show --output=/etc/out.patch"), "ask");
}

#[test]
fn bash_git_diff_output_eq_writable() {
    assert_decision(&bash_input("git diff --output=/tmp/diff.patch"), "allow");
}

#[test]
fn bash_curl_output_eq_writable() {
    // Also test with a non-git option — curl has --output with aliases
    assert_decision(
        &bash_input("curl --request GET --output=/tmp/out.json http://example.com"),
        "allow",
    );
}

#[test]
fn bash_curl_output_eq_not_writable() {
    assert_decision(
        &bash_input("curl --request GET --output=/etc/out.json http://example.com"),
        "deny",
    );
}

#[test]
fn bash_sort_output_eq_writable() {
    assert_decision(
        &bash_input("sort --output=/tmp/sorted.txt input.txt"),
        "allow",
    );
}

#[test]
fn bash_git_pre_subcmd_option_eq() {
    // git -c key=value uses = in the value, but -c is a known option
    // so it should match via direct lookup (not eq splitting)
    assert_decision(&bash_input("git -c core.editor=vim status"), "ask");
}

// =============================================================================
// Bash tool — dotnet output path options
// =============================================================================

#[test]
fn bash_dotnet_build_allowed() {
    assert_decision(&bash_input("dotnet build"), "allow");
}

#[test]
fn bash_dotnet_build_output_writable() {
    assert_decision(&bash_input("dotnet build -o /tmp/out"), "allow");
}

#[test]
fn bash_dotnet_build_output_not_writable() {
    assert_decision(&bash_input("dotnet build -o /etc/out"), "ask");
}

#[test]
fn bash_dotnet_build_artifacts_path_writable() {
    assert_decision(
        &bash_input("dotnet build --artifacts-path /tmp/artifacts"),
        "allow",
    );
}

#[test]
fn bash_dotnet_test_results_dir_writable() {
    assert_decision(
        &bash_input("dotnet test --results-directory /tmp/results"),
        "allow",
    );
}

#[test]
fn bash_dotnet_test_results_dir_not_writable() {
    assert_decision(
        &bash_input("dotnet test --results-directory /etc/results"),
        "ask",
    );
}

#[test]
fn bash_dotnet_restore_packages_writable() {
    assert_decision(&bash_input("dotnet restore --packages /tmp/pkgs"), "allow");
}

// =============================================================================
// Bash tool — cargo --target-dir
// =============================================================================

#[test]
fn bash_cargo_build_allowed() {
    assert_decision(&bash_input("cargo build"), "allow");
}

#[test]
fn bash_cargo_build_target_dir_writable() {
    assert_decision(&bash_input("cargo build --target-dir /tmp/target"), "allow");
}

#[test]
fn bash_cargo_build_target_dir_not_writable() {
    // /etc/target is readable but not writable → ask
    assert_decision(&bash_input("cargo build --target-dir /etc/target"), "ask");
}

#[test]
fn bash_cargo_fmt_no_target_dir() {
    // fmt is a plain allow — doesn't have --target-dir
    assert_decision(&bash_input("cargo fmt"), "allow");
}

// =============================================================================
// Bash tool — nix eval --write-to
// =============================================================================

#[test]
fn bash_nix_eval_allowed() {
    assert_decision(&bash_input("nix eval .#foo"), "allow");
}

#[test]
fn bash_nix_eval_write_to_writable() {
    assert_decision(&bash_input("nix eval --write-to /tmp/out .#foo"), "allow");
}

#[test]
fn bash_nix_eval_write_to_not_writable() {
    // /etc/out is readable but not writable → ask
    assert_decision(&bash_input("nix eval --write-to /etc/out .#foo"), "ask");
}

// =============================================================================
// Bash tool — rm -fr denied
// =============================================================================

#[test]
fn bash_rm_fr_denied() {
    // -fr is the reversed form of -rf; the hook treats combined flags as
    // single tokens without decomposing them, so it needs an explicit entry.
    assert_decision(&bash_input("rm -fr /tmp/file.txt"), "deny");
}

// =============================================================================
// Error cases
// =============================================================================

#[test]
fn malformed_json_ask() {
    let (code, decision, _) = run_hook("not json at all");
    assert_eq!(code, 0);
    assert_eq!(decision, "ask");
}

#[test]
fn missing_tool_name_ask() {
    let (code, decision, _) = run_hook(r#"{"tool_input": {}}"#);
    assert_eq!(code, 0);
    assert_eq!(decision, "ask");
}

#[test]
fn missing_rules_file_exits_1() {
    let bin = env!("CARGO_BIN_EXE_claude-pretool-hook");
    let output = cmd!(bin, "--mode", "claude", "--rules", "nonexistent.json")
        .stdin_bytes(&b""[..])
        .stdout_capture()
        .stderr_capture()
        .unchecked()
        .run()
        .unwrap();
    assert_eq!(output.status.code().unwrap(), 1);
}

#[test]
fn no_rules_arg_exits_2() {
    let bin = env!("CARGO_BIN_EXE_claude-pretool-hook");
    let output = cmd!(bin, "--mode", "claude")
        .stdin_bytes(&b""[..])
        .stdout_capture()
        .stderr_capture()
        .unchecked()
        .run()
        .unwrap();
    // clap exits with code 2 for missing required arguments
    assert_eq!(output.status.code().unwrap(), 2);
}

// =============================================================================
// Bash tool — flag positional overlay
// =============================================================================

#[test]
fn bash_flag_positional_no_flag_readable() {
    // patch /tmp/file.txt — no -i, base positional "*" readable check → allow
    assert_decision(&bash_input("patch /tmp/file.txt"), "allow");
}

#[test]
fn bash_flag_positional_no_flag_unreadable() {
    // patch /tmp/app.secret — no -i, file denied by read pattern → deny
    assert_decision(&bash_input("patch /tmp/app.secret"), "deny");
}

#[test]
fn bash_flag_positional_with_flag_writable() {
    // patch -i /tmp/file.txt — base readable + overlay writable, /tmp/ is both → allow
    assert_decision(&bash_input("patch -i /tmp/file.txt"), "allow");
}

#[test]
fn bash_flag_positional_with_flag_not_writable() {
    // patch -i /etc/file.txt — readable but not writable → ask (from overlay)
    assert_decision(&bash_input("patch -i /etc/file.txt"), "ask");
}

#[test]
fn bash_flag_positional_alias() {
    // patch -i (no alias defined in fixture, but -i is the primary key)
    // Verify -i works with a writable path → allow
    assert_decision(&bash_input("patch -i /tmp/file.txt"), "allow");
}

#[test]
fn bash_flag_positional_alias_not_writable() {
    // patch -i with not-writable path → ask
    assert_decision(&bash_input("patch -i /etc/file.txt"), "ask");
}

#[test]
fn bash_flag_positional_wildcard_checks_all_files() {
    // patch -i with two files: both writable → allow
    assert_decision(&bash_input("patch -i /tmp/a.txt /tmp/b.txt"), "allow");
}

#[test]
fn bash_flag_positional_wildcard_one_not_writable() {
    // patch -i with two files: first writable, second not → ask
    assert_decision(&bash_input("patch -i /tmp/a.txt /etc/b.txt"), "ask");
}

// =============================================================================
// Bash tool — cwdCheck
// =============================================================================

#[test]
fn bash_cwd_check_writable_cwd() {
    // unzip /tmp/archive.zip from a writable cwd (/tmp/project) →
    // archive readable + cwd writable → allow
    assert_decision(
        &bash_input_with_cwd("unzip /tmp/archive.zip", "/tmp/project"),
        "allow",
    );
}

#[test]
fn bash_cwd_check_non_writable_cwd() {
    // unzip /tmp/archive.zip from a non-writable cwd (/etc/somewhere) →
    // archive readable + cwd not writable → ask (from cwdCheck ifWritable)
    assert_decision(
        &bash_input_with_cwd("unzip /tmp/archive.zip", "/etc/somewhere"),
        "ask",
    );
}

#[test]
fn bash_cwd_check_read_only_flag() {
    // unzip -l /tmp/archive.zip from a writable cwd → -l is allow,
    // archive readable, cwd writable → allow
    assert_decision(
        &bash_input_with_cwd("unzip -l /tmp/archive.zip", "/tmp/project"),
        "allow",
    );
}

// =============================================================================
// Bash tool — fd: read-only by default, ask on -x/-X (command execution)
// =============================================================================

#[test]
fn bash_fd_basic_allowed() {
    assert_decision(&bash_input("fd pattern"), "allow");
}

#[test]
fn bash_fd_unknown_flag_allowed() {
    // fd has many read-only flags (--hidden, --no-ignore, etc); flag wildcard
    // allows them without enumerating each one
    assert_decision(&bash_input("fd --hidden pattern"), "allow");
}

#[test]
fn bash_fd_exec_short_asks() {
    assert_decision(&bash_input("fd pattern -x echo {}"), "ask");
}

#[test]
fn bash_fd_exec_long_asks() {
    assert_decision(&bash_input("fd pattern --exec echo {}"), "ask");
}

#[test]
fn bash_fd_exec_batch_short_asks() {
    assert_decision(&bash_input("fd pattern -X echo"), "ask");
}

#[test]
fn bash_fd_exec_batch_long_asks() {
    assert_decision(&bash_input("fd pattern --exec-batch echo"), "ask");
}

// =============================================================================
// Bash tool — file: ask on -C (compiles a magic file → writes)
// =============================================================================

#[test]
fn bash_file_basic_allowed() {
    assert_decision(&bash_input("file /tmp/data"), "allow");
}

#[test]
fn bash_file_compile_short_asks() {
    assert_decision(&bash_input("file -C"), "ask");
}

#[test]
fn bash_file_compile_long_asks() {
    assert_decision(&bash_input("file --compile"), "ask");
}

// =============================================================================
// Bash tool — info: -o/--output writes node contents to file
// =============================================================================

#[test]
fn bash_info_basic_allowed() {
    assert_decision(&bash_input("info ls"), "allow");
}

#[test]
fn bash_info_output_writable() {
    assert_decision(&bash_input("info -o /tmp/out.txt ls"), "allow");
}

#[test]
fn bash_info_output_not_writable() {
    // /etc/out.txt is readable but not writable → ask
    assert_decision(&bash_input("info -o /etc/out.txt ls"), "ask");
}

#[test]
fn bash_info_output_long_writable() {
    assert_decision(&bash_input("info --output /tmp/out.txt ls"), "allow");
}

#[test]
fn bash_info_output_eq_not_writable() {
    assert_decision(&bash_input("info --output=/etc/out.txt ls"), "ask");
}

// =============================================================================
// Bash tool — man -P: allow common pagers, ask for arbitrary commands
// =============================================================================

#[test]
fn bash_man_basic_allowed() {
    assert_decision(&bash_input("man ls"), "allow");
}

#[test]
fn bash_man_pager_less_allowed() {
    assert_decision(&bash_input("man -P less ls"), "allow");
}

#[test]
fn bash_man_pager_cat_allowed() {
    assert_decision(&bash_input("man -P cat ls"), "allow");
}

#[test]
fn bash_man_pager_more_allowed() {
    assert_decision(&bash_input("man -P more ls"), "allow");
}

#[test]
fn bash_man_pager_bat_allowed() {
    assert_decision(&bash_input("man -P bat ls"), "allow");
}

#[test]
fn bash_man_pager_unknown_asks() {
    // Arbitrary commands as pagers (e.g. shell injection vector) → ask
    assert_decision(&bash_input("man -P vim ls"), "ask");
}

#[test]
fn bash_man_pager_long_alias_allowed() {
    assert_decision(&bash_input("man --pager less ls"), "allow");
}

#[test]
fn bash_man_pager_eq_unknown_asks() {
    assert_decision(&bash_input("man --pager=sh ls"), "ask");
}

// =============================================================================
// Bash tool — mktemp: positional template / -p must be writable
// =============================================================================

#[test]
fn bash_mktemp_no_args_allowed() {
    // Default writes to $TMPDIR; no positional / option to gate
    assert_decision(&bash_input("mktemp"), "allow");
}

#[test]
fn bash_mktemp_template_writable() {
    assert_decision(&bash_input("mktemp /tmp/foo.XXXXXX"), "allow");
}

#[test]
fn bash_mktemp_template_not_writable() {
    // /etc/foo.XXXXXX is readable but not writable → ask
    assert_decision(&bash_input("mktemp /etc/foo.XXXXXX"), "ask");
}

#[test]
fn bash_mktemp_tmpdir_short_writable() {
    // -p value writable; no template positional → allow
    assert_decision(&bash_input("mktemp -p /tmp/sub"), "allow");
}

#[test]
fn bash_mktemp_tmpdir_short_not_writable() {
    assert_decision(&bash_input("mktemp -p /etc/sub"), "ask");
}

#[test]
fn bash_mktemp_tmpdir_long_eq_writable() {
    assert_decision(&bash_input("mktemp --tmpdir=/tmp/sub"), "allow");
}

// TODO: `mktemp -p /tmp/sub foo.XXXXXX` triggers ask because the relative
// template is checked against cwd as a positional, not against -p's value.
// Same limitation as cp -t / mv -t (positional rules don't get overridden
// by flag context).

// =============================================================================
// Codex support — Bash parity and apply_patch file-access governance
// =============================================================================

#[test]
fn codex_bash_deny_with_extra_fields() {
    let (code, stdout) = run_hook_raw_with_mode("codex", &codex_bash_input("dd"));
    assert_eq!(code, 0);
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(
        parsed["hookSpecificOutput"]["permissionDecision"], "deny",
        "stdout: {stdout}"
    );
}

#[test]
fn codex_apply_patch_add_writable_allow() {
    let patch = "*** Begin Patch\n*** Add File: /tmp/out.rs\n+fn main() {}\n*** End Patch";
    assert_decision(&apply_patch_input(patch), "allow");
}

#[test]
fn codex_apply_patch_update_readable_only_asks() {
    // Readable but outside the write globs -> ask (via the Edit node).
    let patch = "*** Begin Patch\n*** Update File: /home/user/readme.md\n@@\n-a\n+b\n*** End Patch";
    assert_decision(&apply_patch_input(patch), "ask");
}

#[test]
fn codex_apply_patch_unreadable_denies() {
    // Matches the read-glob secret exclusion -> not readable -> deny.
    let patch =
        "*** Begin Patch\n*** Update File: /home/user/api.secret\n@@\n-a\n+b\n*** End Patch";
    assert_decision(&apply_patch_input(patch), "deny");
}

#[test]
fn codex_apply_patch_multi_file_strictest_wins() {
    // One writable add (allow) plus one unreadable delete (deny) -> deny.
    let patch = "*** Begin Patch\n*** Add File: /tmp/ok.rs\n+x\n*** Delete File: /root/server.key\n*** End Patch";
    assert_decision(&apply_patch_input(patch), "deny");
}

#[test]
fn codex_apply_patch_move_to_non_writable_asks() {
    // Update target is writable (allow) but the rename destination is only
    // readable (ask) -> merged to ask.
    let patch =
        "*** Begin Patch\n*** Update File: /tmp/a.rs\n*** Move to: /home/user/b.rs\n@@\n-a\n+b\n*** End Patch";
    assert_decision(&apply_patch_input(patch), "ask");
}

#[test]
fn codex_apply_patch_unparseable_asks() {
    // No recognizable file headers -> fail safe to ask.
    assert_decision(&apply_patch_input("total garbage, no headers"), "ask");
}

#[test]
fn codex_mode_allow_without_codex_fields_abstains() {
    // Codex has no hook-returnable plain "allow" (it rejects `decision:
    // "approve"` and requires an `updatedInput` rewrite for
    // `permissionDecision: "allow"`), so an allow must abstain with an empty
    // object.
    let (_, stdout) = run_hook_raw_with_mode("codex", &bash_input("ls"));
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert!(
        parsed.get("decision").is_none(),
        "Codex allow must omit decision: {stdout}"
    );
    assert!(
        parsed.get("hookSpecificOutput").is_none(),
        "Codex allow must not carry hookSpecificOutput: {stdout}"
    );
}

#[test]
fn codex_deny_emits_hook_specific_output() {
    // Deny uses the same hookSpecificOutput form Claude uses, which Codex also
    // accepts, with a non-empty reason.
    let (_, stdout) = run_hook_raw_with_mode("codex", &codex_bash_input("dd"));
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(
        parsed["hookSpecificOutput"]["permissionDecision"], "deny",
        "stdout: {stdout}"
    );
    assert!(
        parsed["hookSpecificOutput"]["permissionDecisionReason"]
            .as_str()
            .is_some_and(|r| !r.trim().is_empty()),
        "Codex deny reason must be non-empty: {stdout}"
    );
    assert!(parsed.get("decision").is_none(), "stdout: {stdout}");
}

#[test]
fn codex_ask_abstains() {
    // "ask" has no Codex representation; abstain so Codex uses its normal
    // approval flow.
    let (_, stdout) = run_hook_raw_with_mode("codex", &codex_bash_input("sh"));
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert!(
        parsed.get("decision").is_none(),
        "Codex 'ask' must omit decision: {stdout}"
    );
    assert!(
        parsed.get("hookSpecificOutput").is_none(),
        "stdout: {stdout}"
    );
}

#[test]
fn codex_apply_patch_deny_blocks() {
    // An apply_patch touching an unreadable path is denied; under Codex that
    // surfaces as a real hookSpecificOutput deny.
    let patch =
        "*** Begin Patch\n*** Update File: /home/user/api.secret\n@@\n-a\n+b\n*** End Patch";
    let (_, stdout) = run_hook_raw_with_mode("codex", &codex_apply_patch_input(patch));
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(
        parsed["hookSpecificOutput"]["permissionDecision"], "deny",
        "stdout: {stdout}"
    );
}

#[test]
fn claude_output_format_unchanged() {
    // The explicit Claude mode controls the output even when the payload has
    // fields that Codex sends.
    let (_, stdout) = run_hook_raw_with_mode("claude", &codex_bash_input("ls"));
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(
        parsed["hookSpecificOutput"]["permissionDecision"], "allow",
        "stdout: {stdout}"
    );
    assert!(parsed.get("decision").is_none(), "stdout: {stdout}");
}

// =============================================================================
// deferAskInAutoMode
// =============================================================================

/// A Bash invocation carrying an explicit Claude permission mode.
fn bash_input_with_mode(command: &str, permission_mode: &str) -> String {
    serde_json::json!({
        "tool_name": "Bash",
        "tool_input": { "command": command },
        "hook_event_name": "PreToolUse",
        "permission_mode": permission_mode
    })
    .to_string()
}

/// Assert the decision the defer-ask fixture produces, treating an abstain
/// (empty object) as the distinct decision `""`.
fn assert_defer_decision(input: &str, expected: &str) {
    let (code, stdout) = run_hook_raw_with_rules(DEFER_ASK_RULES, input);
    assert_eq!(code, 0, "expected exit 0, got {code}; stdout: {stdout}");
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let decision = parsed
        .pointer("/hookSpecificOutput/permissionDecision")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    assert_eq!(decision, expected, "stdout: {stdout}");
}

#[test]
fn auto_mode_ask_abstains() {
    // In auto mode with deferAskInAutoMode set, an "ask" is withheld so
    // Claude's own judgment decides rather than the user being prompted.
    assert_defer_decision(&bash_input_with_mode("sh", "auto"), "");
}

#[test]
fn auto_mode_allow_still_reported() {
    assert_defer_decision(&bash_input_with_mode("ls", "auto"), "allow");
}

#[test]
fn auto_mode_deny_still_reported() {
    assert_defer_decision(&bash_input_with_mode("dd", "auto"), "deny");
}

#[test]
fn auto_mode_unknown_command_ask_abstains() {
    // The implicit "ask" for an unlisted command defers just like an explicit
    // one — otherwise deferral would only cover rules that spell out "ask".
    assert_defer_decision(&bash_input_with_mode("nope", "auto"), "");
}

#[test]
fn non_auto_modes_still_ask() {
    // Deferral is scoped to auto mode; every other mode still prompts.
    for mode in ["default", "plan", "acceptEdits", "bypassPermissions"] {
        assert_defer_decision(&bash_input_with_mode("sh", mode), "ask");
    }
}

#[test]
fn missing_permission_mode_still_asks() {
    // A payload without permission_mode is not auto mode.
    let input = serde_json::json!({
        "tool_name": "Bash",
        "tool_input": { "command": "sh" }
    })
    .to_string();
    assert_defer_decision(&input, "ask");
}

#[test]
fn auto_mode_asks_when_deferral_not_enabled() {
    // The main fixture omits deferAskInAutoMode, so auto mode changes nothing.
    assert_decision(&bash_input_with_mode("sh", "auto"), "ask");
}

#[test]
fn unparseable_input_asks_even_in_auto_mode() {
    // The permission mode is unknown when the payload does not parse, so the
    // hook falls back to prompting rather than silently deferring.
    let (code, stdout) = run_hook_raw_with_rules(DEFER_ASK_RULES, "{ not json");
    assert_eq!(code, 0, "expected exit 0, got {code}");
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(
        parsed["hookSpecificOutput"]["permissionDecision"], "ask",
        "stdout: {stdout}"
    );
}
