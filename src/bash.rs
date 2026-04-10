/// Bash command parsing using tree-sitter-bash.
///
/// Extracts commands (including those inside command substitutions) and write
/// redirect targets from a bash command string. This mirrors the behavior of
/// the shfmt+jq approach: recursive descent into all nested commands.
use tree_sitter::{Node, Parser};

#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    /// tree-sitter returned None from parse(), which happens only when an
    /// internal timeout fires — should be unreachable under normal conditions.
    #[error("tree-sitter failed to produce a parse tree")]
    TreeSitterFailed,
}

/// A single extracted command with its arguments and expansion tracking.
pub struct ExtractedCommand {
    /// Literal text of each argument (expansions replaced with empty string).
    pub args: Vec<String>,
    /// Per-argument flag: true if the arg contains any non-literal part.
    pub expansion_flags: Vec<bool>,
}

/// A write redirect target (> or >>).
pub struct WriteRedirect {
    pub path: String,
}

/// Parse a bash command string and extract all commands and write redirects.
pub fn parse(command: &str) -> Result<(Vec<ExtractedCommand>, Vec<WriteRedirect>), ParseError> {
    let mut parser = Parser::new();
    let language = tree_sitter_bash::LANGUAGE;
    parser
        .set_language(&language.into())
        .expect("Error loading Bash grammar");

    let tree = parser
        .parse(command, None)
        .ok_or(ParseError::TreeSitterFailed)?;
    let root = tree.root_node();
    let source = command.as_bytes();

    let mut commands = Vec::new();
    let mut redirects = Vec::new();

    collect_commands(root, source, &mut commands);
    collect_redirects(root, source, &mut redirects);

    Ok((commands, redirects))
}

/// Recursively collect all simple_command nodes from the tree, including those
/// inside command substitutions — this is what makes `git commit -m "$(rm -rf /)"``
/// evaluate both `git commit -m ...` and `rm -rf /` independently.
fn collect_commands(node: Node, source: &[u8], out: &mut Vec<ExtractedCommand>) {
    if node.kind() == "command" {
        if let Some(cmd) = extract_command(node, source) {
            out.push(cmd);
        }
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_commands(child, source, out);
    }
}

/// Extract args and expansion flags from a simple_command node.
fn extract_command(node: Node, source: &[u8]) -> Option<ExtractedCommand> {
    let mut args = Vec::new();
    let mut expansion_flags = Vec::new();

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            // Skip redirects — they're handled separately.
            "file_redirect" | "heredoc_redirect" | "herestring_redirect" => continue,
            // command_name is the first arg, then word/string/etc. are the rest.
            "command_name"
            | "word"
            | "concatenation"
            | "string"
            | "raw_string"
            | "simple_expansion"
            | "expansion"
            | "number"
            | "command_substitution" => {
                let (text, has_expansion) = extract_word(child, source);
                args.push(text);
                expansion_flags.push(has_expansion);
            }
            _ => {}
        }
    }

    if args.is_empty() {
        return None;
    }

    Some(ExtractedCommand {
        args,
        expansion_flags,
    })
}

/// Extract the literal text and expansion status from a word-like node.
/// Returns (literal_text, has_expansion).
fn extract_word(node: Node, source: &[u8]) -> (String, bool) {
    match node.kind() {
        "word" | "command_name" => {
            // A bare word with no children — it's fully literal.
            if node.child_count() == 0 {
                let text = node_text(node, source);
                return (text, false);
            }
            // Composite word: concatenation of literal and non-literal parts.
            let mut text = String::new();
            let mut has_expansion = false;
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                let (part_text, part_exp) = extract_word(child, source);
                text.push_str(&part_text);
                has_expansion |= part_exp;
            }
            (text, has_expansion)
        }
        "concatenation" => {
            let mut text = String::new();
            let mut has_expansion = false;
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                let (part_text, part_exp) = extract_word(child, source);
                text.push_str(&part_text);
                has_expansion |= part_exp;
            }
            (text, has_expansion)
        }
        "raw_string" => {
            // $'...' or '...' — fully literal.
            let text = node_text(node, source);
            // Strip surrounding quotes.
            let inner = if text.starts_with('\'') && text.ends_with('\'') && text.len() >= 2 {
                text[1..text.len() - 1].to_string()
            } else {
                text
            };
            (inner, false)
        }
        "string" => {
            // "..." — may contain expansions.
            let mut text = String::new();
            let mut has_expansion = false;
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                match child.kind() {
                    "\"" => {} // Skip quote delimiters.
                    "string_content" => {
                        text.push_str(&node_text(child, source));
                    }
                    "simple_expansion" | "expansion" | "command_substitution" => {
                        has_expansion = true;
                        // Don't append text — expansions contribute empty string.
                    }
                    _ => {
                        let (part_text, part_exp) = extract_word(child, source);
                        text.push_str(&part_text);
                        has_expansion |= part_exp;
                    }
                }
            }
            (text, has_expansion)
        }
        // Non-literal nodes: expansions, substitutions.
        "simple_expansion" | "expansion" | "command_substitution" => (String::new(), true),
        "number" => {
            let text = node_text(node, source);
            (text, false)
        }
        _ => {
            // Fallback: treat as literal text.
            let text = node_text(node, source);
            (text, false)
        }
    }
}

/// Recursively collect write redirect targets (> and >>).
fn collect_redirects(node: Node, source: &[u8], out: &mut Vec<WriteRedirect>) {
    if node.kind() == "file_redirect" {
        if let Some(redirect) = extract_redirect(node, source) {
            out.push(redirect);
        }
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_redirects(child, source, out);
    }
}

/// Extract a write redirect target from a file_redirect node.
/// Only captures > and >> (not < or other redirects).
fn extract_redirect(node: Node, source: &[u8]) -> Option<WriteRedirect> {
    let mut is_write = false;
    let mut target = None;

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            ">" | ">>" => {
                is_write = true;
            }
            _ if is_write && target.is_none() => {
                let (text, _) = extract_word(child, source);
                if !text.is_empty() {
                    target = Some(text);
                }
            }
            _ => {}
        }
    }

    if is_write {
        target.map(|path| WriteRedirect { path })
    } else {
        None
    }
}

fn node_text(node: Node, source: &[u8]) -> String {
    node.utf8_text(source).unwrap_or("").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: parse and return just commands (no redirects).
    fn parse_cmds(input: &str) -> Vec<ExtractedCommand> {
        parse(input).expect("parse should succeed").0
    }

    /// Helper: parse and return just redirects.
    fn parse_redirects(input: &str) -> Vec<WriteRedirect> {
        parse(input).expect("parse should succeed").1
    }

    // --- Simple commands ---

    #[test]
    fn simple_command() {
        let cmds = parse_cmds("safe-read foo bar");
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].args, vec!["safe-read", "foo", "bar"]);
        assert_eq!(cmds[0].expansion_flags, vec![false, false, false]);
    }

    #[test]
    fn command_with_flags() {
        let cmds = parse_cmds("build --verbose --force");
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].args, vec!["build", "--verbose", "--force"]);
    }

    // --- Pipes ---

    #[test]
    fn piped_commands() {
        let cmds = parse_cmds("safe-read foo | inspect status");
        assert_eq!(cmds.len(), 2);
        assert_eq!(cmds[0].args, vec!["safe-read", "foo"]);
        assert_eq!(cmds[1].args, vec!["inspect", "status"]);
    }

    // --- Command substitution ---

    #[test]
    fn command_substitution_extracted() {
        let cmds = parse_cmds("configure --env $(safe-read envfile)");
        // Should extract both the outer command and the inner substitution
        assert!(cmds.len() >= 2);
        // Inner command
        let inner = cmds.iter().find(|c| c.args[0] == "safe-read");
        assert!(inner.is_some());
    }

    #[test]
    fn command_substitution_marks_expansion() {
        let cmds = parse_cmds("configure --env $(safe-read envfile)");
        let outer = &cmds[0];
        // The arg containing $(…) should be marked as having an expansion
        assert!(outer.expansion_flags.iter().any(|&f| f));
    }

    // --- Variable expansions ---

    #[test]
    fn dollar_var_marks_expansion() {
        let cmds = parse_cmds("process $MY_VAR");
        assert_eq!(cmds.len(), 1);
        assert!(cmds[0].expansion_flags[1]);
    }

    #[test]
    fn brace_expansion_marks_expansion() {
        let cmds = parse_cmds("process ${MY_VAR}");
        assert_eq!(cmds.len(), 1);
        assert!(cmds[0].expansion_flags[1]);
    }

    #[test]
    fn quoted_string_with_expansion() {
        let cmds = parse_cmds(r#"process "hello $NAME""#);
        assert_eq!(cmds.len(), 1);
        // The quoted arg contains an expansion
        assert!(cmds[0].expansion_flags[1]);
    }

    #[test]
    fn quoted_string_without_expansion() {
        let cmds = parse_cmds(r#"process "hello world""#);
        assert_eq!(cmds.len(), 1);
        assert!(!cmds[0].expansion_flags[1]);
        assert_eq!(cmds[0].args[1], "hello world");
    }

    // --- Raw strings ---

    #[test]
    fn single_quoted_is_literal() {
        let cmds = parse_cmds("process 'no $expansion here'");
        assert_eq!(cmds.len(), 1);
        assert!(!cmds[0].expansion_flags[1]);
        assert_eq!(cmds[0].args[1], "no $expansion here");
    }

    // --- Redirects ---

    #[test]
    fn write_redirect_captured() {
        let redirects = parse_redirects("safe-read foo > /tmp/out.txt");
        assert_eq!(redirects.len(), 1);
        assert_eq!(redirects[0].path, "/tmp/out.txt");
    }

    #[test]
    fn append_redirect_captured() {
        let redirects = parse_redirects("safe-read foo >> /tmp/out.txt");
        assert_eq!(redirects.len(), 1);
        assert_eq!(redirects[0].path, "/tmp/out.txt");
    }

    #[test]
    fn no_redirect_for_input() {
        let redirects = parse_redirects("process < /dev/null");
        assert!(redirects.is_empty());
    }

    // --- Chained commands ---

    #[test]
    fn and_chain() {
        let cmds = parse_cmds("safe-read foo && inspect status");
        assert_eq!(cmds.len(), 2);
    }

    #[test]
    fn semicolon_chain() {
        let cmds = parse_cmds("safe-read foo; inspect status");
        assert_eq!(cmds.len(), 2);
    }

    // --- Edge cases ---

    #[test]
    fn empty_input() {
        let (cmds, redirects) = parse("").unwrap();
        // Empty string may parse to 0 commands
        assert!(cmds.is_empty());
        assert!(redirects.is_empty());
    }

    #[test]
    fn number_arg() {
        let cmds = parse_cmds("run-wrapper --bg 42 safe-read");
        let args: Vec<&str> = cmds[0].args.iter().map(|s| s.as_str()).collect();
        assert!(args.contains(&"42"));
    }
}
