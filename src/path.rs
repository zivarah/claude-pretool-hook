use std::path::{Path, PathBuf};

use globset::{Glob, GlobMatcher};
use path_clean::PathClean;

use crate::{
    decision::{Condition, ConditionalBranch, ConditionalDecisionNode, Decision, DecisionNode},
    rules::FileAccess,
};

#[derive(Debug, thiserror::Error)]
pub enum PathError {
    /// Path bytes cannot be represented as UTF-8, so no glob matching is possible.
    #[error("path is not valid UTF-8")]
    NotUtf8,
    /// A glob pattern in the rules file failed to compile.
    #[error("invalid glob pattern {pattern:?}: {source}")]
    InvalidGlob {
        pattern: String,
        #[source]
        source: globset::Error,
    },
}

/// Pre-compiled glob patterns for efficient repeated matching.
/// Each entry is (negated, matcher).
struct CompiledPatterns(Vec<(bool, GlobMatcher)>);

impl CompiledPatterns {
    fn compile(patterns: &[String]) -> Result<Self, PathError> {
        patterns
            .iter()
            .map(|pattern| {
                let (negated, glob_str) = match pattern.strip_prefix('!') {
                    Some(rest) => (true, rest),
                    None => (false, pattern.as_str()),
                };
                let glob = Glob::new(glob_str).map_err(|e| PathError::InvalidGlob {
                    pattern: glob_str.to_owned(),
                    source: e,
                })?;
                Ok((negated, glob.compile_matcher()))
            })
            .collect::<Result<Vec<_>, _>>()
            .map(CompiledPatterns)
    }

    /// Evaluate patterns in order (last match wins, negated = non-match).
    /// Returns `false` if no pattern matches.
    fn matches(&self, filepath: &str, cwd: &Path) -> Result<bool, PathError> {
        let normalized = normalize_path(filepath, cwd)?;
        let mut result = false;
        for (negated, matcher) in &self.0 {
            if matcher.is_match(&normalized) {
                result = !negated;
            }
        }
        Ok(result)
    }
}

/// Pre-compiled file access rules. Built once from `FileAccess` at startup.
/// Mirrors the structure of `FileAccess` for clarity.
pub struct CompiledFileAccess {
    pub read: CompiledAccessRules,
    pub write: CompiledWriteRules,
}

pub struct CompiledAccessRules {
    patterns: CompiledPatterns,
}

pub struct CompiledWriteRules {
    patterns: CompiledPatterns,
    require_readable: bool,
}

impl CompiledFileAccess {
    /// Compile all glob patterns from `FileAccess`. When `project_dir` is
    /// provided, `<dir>/**` is prepended to both the read and write lists
    /// so that user-supplied negations (which come after, and win under
    /// last-match-wins) can still carve exceptions out of the project dir.
    pub fn compile(
        file_access: &FileAccess,
        cwd: &Path,
        project_dir: Option<&str>,
    ) -> Result<Self, PathError> {
        let mut read_patterns = file_access.read.glob_patterns.clone();
        let mut write_patterns = file_access.write.glob_patterns.clone();

        if let Some(dir) = project_dir {
            if let Ok(mut normalized) = normalize_path(dir, cwd) {
                if !normalized.ends_with('/') {
                    normalized.push('/');
                }
                read_patterns.insert(0, format!("{normalized}**"));
                write_patterns.insert(0, format!("{normalized}**"));
            }
        }

        let read = CompiledAccessRules {
            patterns: CompiledPatterns::compile(&read_patterns)?,
        };

        let write = CompiledWriteRules {
            patterns: CompiledPatterns::compile(&write_patterns)?,
            require_readable: file_access.write.require_readable,
        };

        Ok(CompiledFileAccess { read, write })
    }
}

/// Normalize a path — make it absolute (prepend `cwd` if relative) and
/// lexically resolve `.` / `..` components (no filesystem access).
/// Returns an error if the result is not valid UTF-8.
pub fn normalize_path(path: &str, cwd: &Path) -> Result<String, PathError> {
    let p = PathBuf::from(path);
    let absolute = if p.is_absolute() { p } else { cwd.join(p) };
    absolute
        .clean()
        .to_str()
        .map(|s| s.to_owned())
        .ok_or(PathError::NotUtf8)
}

/// Check if a path is readable according to compiled file access rules.
pub fn is_readable(filepath: &str, fa: &CompiledFileAccess, cwd: &Path) -> Result<bool, PathError> {
    fa.read.patterns.matches(filepath, cwd)
}

/// Result of attempting to read a file for a `checkFile` content scan.
pub enum FileCheckRead {
    /// File was successfully read. Contents are within the size cap and
    /// valid UTF-8.
    Contents(String),
    /// File could not be read for any reason — blocked by read globs,
    /// missing on disk, oversized, not valid UTF-8, or a generic I/O
    /// error. The inner string is a short human-readable explanation
    /// suitable for inclusion in an `onUnreadable` judgment's reason.
    Unreadable(String),
}

/// Production default size cap for `read_for_check`. Files larger than
/// this are reported as `Unreadable("...exceeds N byte limit")`. Tests
/// pass a smaller cap to exercise the oversize path without writing a
/// megabyte to disk.
pub const FILE_CHECK_MAX_BYTES: usize = 1024 * 1024;

/// Read a file's contents for `checkFile` evaluation. The path is first
/// gated by `is_readable`, so a path blocked by the read globs is
/// reported as unreadable without any filesystem access. On success the
/// file's UTF-8 contents are returned, capped at `max_bytes`. Files
/// exceeding the cap, missing on disk, not valid UTF-8, or hitting an
/// I/O error all map to `Unreadable(...)` so the caller can emit a
/// single `onUnreadable` judgment regardless of why the read failed.
pub fn read_for_check(
    filepath: &str,
    fa: &CompiledFileAccess,
    cwd: &Path,
    max_bytes: usize,
) -> Result<FileCheckRead, PathError> {
    if !is_readable(filepath, fa, cwd)? {
        return Ok(FileCheckRead::Unreadable(format!(
            "path '{}' is not readable per file access rules",
            filepath
        )));
    }

    let normalized = normalize_path(filepath, cwd)?;
    let file = match std::fs::File::open(&normalized) {
        Ok(f) => f,
        Err(err) => {
            return Ok(FileCheckRead::Unreadable(format!(
                "cannot open '{}': {}",
                normalized, err
            )));
        }
    };

    use std::io::Read;
    let mut buf = Vec::with_capacity(max_bytes.min(64 * 1024));
    let to_read = (max_bytes as u64).saturating_add(1);
    if let Err(err) = file.take(to_read).read_to_end(&mut buf) {
        return Ok(FileCheckRead::Unreadable(format!(
            "read error on '{}': {}",
            normalized, err
        )));
    }
    if buf.len() > max_bytes {
        return Ok(FileCheckRead::Unreadable(format!(
            "file '{}' exceeds {} byte limit",
            normalized, max_bytes
        )));
    }
    match String::from_utf8(buf) {
        Ok(s) => Ok(FileCheckRead::Contents(s)),
        Err(_) => Ok(FileCheckRead::Unreadable(format!(
            "file '{}' is not valid UTF-8",
            normalized
        ))),
    }
}

/// Check if a path is writable according to compiled file access rules.
/// When `require_readable` is set, a path must also pass the read globs.
pub fn is_writable(filepath: &str, fa: &CompiledFileAccess, cwd: &Path) -> Result<bool, PathError> {
    if fa.write.require_readable && !is_readable(filepath, fa, cwd)? {
        return Ok(false);
    }
    fa.write.patterns.matches(filepath, cwd)
}

/// Resolve a `DecisionNode` against a path using compiled file access rules.
/// `Static` decisions are returned directly; `Conditional` decisions are
/// evaluated by checking the path against the file-access glob patterns.
pub fn resolve_conditional(
    node: &DecisionNode,
    path: &str,
    fa: &CompiledFileAccess,
    cwd: &Path,
) -> Result<Decision, PathError> {
    match node {
        DecisionNode::Static(d) => Ok(*d),
        DecisionNode::Conditional(c) => resolve_node(c, path, fa, cwd),
    }
}

fn resolve_node(
    node: &ConditionalDecisionNode,
    path: &str,
    fa: &CompiledFileAccess,
    cwd: &Path,
) -> Result<Decision, PathError> {
    let passed = match node.condition {
        Condition::Readable => is_readable(path, fa, cwd)?,
        Condition::Writable => is_writable(path, fa, cwd)?,
    };
    let branch = if passed {
        &node.then_decision
    } else {
        &node.else_decision
    };
    resolve_branch(branch, path, fa, cwd)
}

fn resolve_branch(
    branch: &ConditionalBranch,
    path: &str,
    fa: &CompiledFileAccess,
    cwd: &Path,
) -> Result<Decision, PathError> {
    match branch {
        ConditionalBranch::Static(d) => Ok(*d),
        ConditionalBranch::Nested(c) => resolve_node(c, path, fa, cwd),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// All tests use absolute paths so cwd choice doesn't affect results.
    fn cwd() -> PathBuf {
        PathBuf::from("/test/cwd")
    }

    fn test_file_access() -> CompiledFileAccess {
        CompiledFileAccess::compile(
            &FileAccess {
                read: crate::rules::AccessRules {
                    glob_patterns: vec!["**".into(), "!**/*.secret*".into(), "!**/*.key*".into()],
                },
                write: crate::rules::WriteRules {
                    glob_patterns: vec!["/tmp/**".into(), "/workspace/**".into()],
                    require_readable: true,
                },
            },
            &cwd(),
            None,
        )
        .unwrap()
    }

    // --- normalize_path ---

    #[test]
    fn normalize_absolute_unchanged() {
        assert_eq!(
            normalize_path("/usr/bin/foo", &cwd()).unwrap(),
            "/usr/bin/foo"
        );
    }

    #[test]
    fn normalize_dotdot() {
        assert_eq!(
            normalize_path("/usr/bin/../lib", &cwd()).unwrap(),
            "/usr/lib"
        );
    }

    #[test]
    fn normalize_dot() {
        assert_eq!(normalize_path("/usr/./bin", &cwd()).unwrap(), "/usr/bin");
    }

    #[test]
    fn normalize_dotdot_at_root() {
        assert_eq!(normalize_path("/../foo", &cwd()).unwrap(), "/foo");
    }

    #[test]
    fn normalize_trailing_slash_dotdot() {
        assert_eq!(normalize_path("/a/b/c/../../d", &cwd()).unwrap(), "/a/d");
    }

    #[test]
    fn normalize_relative_uses_cwd() {
        let cwd = PathBuf::from("/home/user/project");
        assert_eq!(
            normalize_path("src/main.rs", &cwd).unwrap(),
            "/home/user/project/src/main.rs"
        );
    }

    // --- is_readable ---

    #[test]
    fn readable_normal_path() {
        let fa = test_file_access();
        assert!(is_readable("/home/user/readme.md", &fa, &cwd()).unwrap());
    }

    #[test]
    fn readable_denied_secret() {
        let fa = test_file_access();
        assert!(!is_readable("/home/user/.secret", &fa, &cwd()).unwrap());
    }

    #[test]
    fn readable_denied_key() {
        let fa = test_file_access();
        assert!(!is_readable("/etc/api.key", &fa, &cwd()).unwrap());
    }

    #[test]
    fn readable_denied_substring() {
        let fa = test_file_access();
        assert!(!is_readable("/etc/api.key.bak", &fa, &cwd()).unwrap());
    }

    // --- is_writable ---

    #[test]
    fn writable_within_prefix() {
        let fa = test_file_access();
        assert!(is_writable("/tmp/output.txt", &fa, &cwd()).unwrap());
    }

    #[test]
    fn writable_within_workspace() {
        let fa = test_file_access();
        assert!(is_writable("/workspace/src/main.rs", &fa, &cwd()).unwrap());
    }

    #[test]
    fn writable_outside_prefix() {
        let fa = test_file_access();
        assert!(!is_writable("/etc/passwd", &fa, &cwd()).unwrap());
    }

    #[test]
    fn writable_denied_even_in_prefix() {
        let fa = test_file_access();
        assert!(!is_writable("/tmp/.secret", &fa, &cwd()).unwrap());
    }

    #[test]
    fn writable_require_readable_blocks_unreadable() {
        let fa = CompiledFileAccess::compile(
            &FileAccess {
                read: crate::rules::AccessRules {
                    glob_patterns: vec!["**".into(), "!**/blocked".into()],
                },
                write: crate::rules::WriteRules {
                    glob_patterns: vec!["/tmp/**".into()],
                    require_readable: true,
                },
            },
            &cwd(),
            None,
        )
        .unwrap();
        assert!(is_writable("/tmp/ok", &fa, &cwd()).unwrap());
        assert!(!is_writable("/tmp/blocked", &fa, &cwd()).unwrap());
    }

    #[test]
    fn writable_without_require_readable_allows_unreadable() {
        let fa = CompiledFileAccess::compile(
            &FileAccess {
                read: crate::rules::AccessRules {
                    glob_patterns: vec!["**".into(), "!**/blocked".into()],
                },
                write: crate::rules::WriteRules {
                    glob_patterns: vec!["/tmp/**".into()],
                    require_readable: false,
                },
            },
            &cwd(),
            None,
        )
        .unwrap();
        assert!(is_writable("/tmp/blocked", &fa, &cwd()).unwrap());
    }

    #[test]
    fn writable_project_dir() {
        let fa = CompiledFileAccess::compile(
            &FileAccess {
                read: crate::rules::AccessRules {
                    glob_patterns: vec!["**".into()],
                },
                write: crate::rules::WriteRules {
                    glob_patterns: vec!["/tmp/**".into()],
                    require_readable: true,
                },
            },
            &cwd(),
            Some("/my/project"),
        )
        .unwrap();
        assert!(is_writable("/my/project/src/lib.rs", &fa, &cwd()).unwrap());
        assert!(!is_writable("/other/path", &fa, &cwd()).unwrap());
    }

    #[test]
    fn readable_project_dir_added() {
        // With no user read patterns, the project dir alone makes files
        // inside it readable.
        let fa = CompiledFileAccess::compile(
            &FileAccess {
                read: crate::rules::AccessRules {
                    glob_patterns: vec![],
                },
                write: crate::rules::WriteRules {
                    glob_patterns: vec![],
                    require_readable: false,
                },
            },
            &cwd(),
            Some("/my/project"),
        )
        .unwrap();
        assert!(is_readable("/my/project/src/lib.rs", &fa, &cwd()).unwrap());
        assert!(!is_readable("/other/path", &fa, &cwd()).unwrap());
    }

    #[test]
    fn user_read_negation_overrides_project_dir() {
        // Project dir is prepended; the user's negation comes later in the
        // list and wins under last-match-wins semantics.
        let fa = CompiledFileAccess::compile(
            &FileAccess {
                read: crate::rules::AccessRules {
                    glob_patterns: vec!["!**/*.secret*".into()],
                },
                write: crate::rules::WriteRules {
                    glob_patterns: vec![],
                    require_readable: false,
                },
            },
            &cwd(),
            Some("/my/project"),
        )
        .unwrap();
        // Files inside the project dir but matching the user negation are
        // NOT readable — the negation overrides the auto-added project dir.
        assert!(!is_readable("/my/project/api.secret", &fa, &cwd()).unwrap());
        // Other files in the project dir are still readable.
        assert!(is_readable("/my/project/src/lib.rs", &fa, &cwd()).unwrap());
    }

    #[test]
    fn user_write_negation_overrides_project_dir() {
        // Same as above, but for write patterns.
        let fa = CompiledFileAccess::compile(
            &FileAccess {
                read: crate::rules::AccessRules {
                    glob_patterns: vec!["**".into()],
                },
                write: crate::rules::WriteRules {
                    glob_patterns: vec!["!**/dist/**".into()],
                    require_readable: false,
                },
            },
            &cwd(),
            Some("/my/project"),
        )
        .unwrap();
        // Files matching the user negation are NOT writable, even though
        // they're inside the auto-added project dir.
        assert!(!is_writable("/my/project/dist/bundle.js", &fa, &cwd()).unwrap());
        // Other files in the project dir are still writable.
        assert!(is_writable("/my/project/src/lib.rs", &fa, &cwd()).unwrap());
    }

    #[test]
    fn user_negation_then_re_allow_overrides_project_dir() {
        // Verifies the project dir is prepended (not appended): a user
        // negation followed by a re-allow on the same path produces the
        // re-allow as the final result. If the project dir were appended
        // instead, the project-dir glob would be the last match for any
        // path under it and the user re-allow would have no effect on
        // anything outside the project dir — but more importantly, this
        // exercises the ordering: user patterns must come *after* the
        // project-dir auto-add so that a user "!**/*.tmp*" can still be
        // overridden by a later user "**/keep.tmp" entry.
        let fa = CompiledFileAccess::compile(
            &FileAccess {
                read: crate::rules::AccessRules {
                    glob_patterns: vec!["!**/*.tmp*".into(), "**/keep.tmp".into()],
                },
                write: crate::rules::WriteRules {
                    glob_patterns: vec![],
                    require_readable: false,
                },
            },
            &cwd(),
            Some("/my/project"),
        )
        .unwrap();
        assert!(is_readable("/my/project/keep.tmp", &fa, &cwd()).unwrap());
        assert!(!is_readable("/my/project/scratch.tmp", &fa, &cwd()).unwrap());
    }

    // --- CompiledPatterns ---

    #[test]
    fn patterns_last_match_wins() {
        let patterns: Vec<String> =
            vec!["**".into(), "!**/*.txt".into(), "**/important.txt".into()];
        let compiled = CompiledPatterns::compile(&patterns).unwrap();
        assert!(compiled.matches("/foo/important.txt", &cwd()).unwrap());
        assert!(!compiled.matches("/foo/other.txt", &cwd()).unwrap());
        assert!(compiled.matches("/foo/readme.md", &cwd()).unwrap());
    }

    #[test]
    fn patterns_no_match_is_false() {
        let patterns: Vec<String> = vec!["/tmp/**".into()];
        let compiled = CompiledPatterns::compile(&patterns).unwrap();
        assert!(!compiled.matches("/etc/foo", &cwd()).unwrap());
    }

    // --- resolve_conditional ---

    fn readable_node(then: Decision, else_: Decision) -> DecisionNode {
        DecisionNode::Conditional(Box::new(ConditionalDecisionNode {
            condition: Condition::Readable,
            then_decision: ConditionalBranch::Static(then),
            else_decision: ConditionalBranch::Static(else_),
        }))
    }

    fn writable_node(then: Decision, else_: Decision) -> DecisionNode {
        DecisionNode::Conditional(Box::new(ConditionalDecisionNode {
            condition: Condition::Writable,
            then_decision: ConditionalBranch::Static(then),
            else_decision: ConditionalBranch::Static(else_),
        }))
    }

    #[test]
    fn resolve_plain_decision() {
        let fa = test_file_access();
        let d = DecisionNode::Static(Decision::Allow);
        assert_eq!(
            resolve_conditional(&d, "/any/path", &fa, &cwd()).unwrap(),
            Decision::Allow
        );
    }

    #[test]
    fn resolve_readable_passes_for_clean_path() {
        let fa = test_file_access();
        let d = readable_node(Decision::Allow, Decision::Deny);
        assert_eq!(
            resolve_conditional(&d, "/home/user/readme.md", &fa, &cwd()).unwrap(),
            Decision::Allow
        );
    }

    #[test]
    fn resolve_readable_fails_for_denied_path() {
        let fa = test_file_access();
        let d = readable_node(Decision::Allow, Decision::Deny);
        assert_eq!(
            resolve_conditional(&d, "/home/user/.secret", &fa, &cwd()).unwrap(),
            Decision::Deny
        );
    }

    #[test]
    fn resolve_writable_passes_for_allowed_prefix() {
        let fa = test_file_access();
        let d = writable_node(Decision::Allow, Decision::Ask);
        assert_eq!(
            resolve_conditional(&d, "/tmp/out.txt", &fa, &cwd()).unwrap(),
            Decision::Allow
        );
    }

    #[test]
    fn resolve_writable_fails_for_denied_basename() {
        let fa = test_file_access();
        let d = writable_node(Decision::Allow, Decision::Deny);
        assert_eq!(
            resolve_conditional(&d, "/tmp/.secret", &fa, &cwd()).unwrap(),
            Decision::Deny
        );
    }

    #[test]
    fn resolve_writable_fails_outside_prefix() {
        let fa = test_file_access();
        let d = writable_node(Decision::Allow, Decision::Ask);
        assert_eq!(
            resolve_conditional(&d, "/etc/config.txt", &fa, &cwd()).unwrap(),
            Decision::Ask
        );
    }

    #[test]
    fn resolve_nested_conditional_on_then_branch() {
        let fa = test_file_access();
        let d = DecisionNode::Conditional(Box::new(ConditionalDecisionNode {
            condition: Condition::Readable,
            then_decision: ConditionalBranch::Nested(Box::new(ConditionalDecisionNode {
                condition: Condition::Writable,
                then_decision: ConditionalBranch::Static(Decision::Allow),
                else_decision: ConditionalBranch::Static(Decision::Ask),
            })),
            else_decision: ConditionalBranch::Static(Decision::Deny),
        }));
        assert_eq!(
            resolve_conditional(&d, "/tmp/file.txt", &fa, &cwd()).unwrap(),
            Decision::Allow
        );
        assert_eq!(
            resolve_conditional(&d, "/etc/config.txt", &fa, &cwd()).unwrap(),
            Decision::Ask
        );
        assert_eq!(
            resolve_conditional(&d, "/etc/.secret", &fa, &cwd()).unwrap(),
            Decision::Deny
        );
    }

    #[test]
    fn resolve_nested_conditional() {
        let fa = test_file_access();
        let d = DecisionNode::Conditional(Box::new(ConditionalDecisionNode {
            condition: Condition::Writable,
            then_decision: ConditionalBranch::Static(Decision::Allow),
            else_decision: ConditionalBranch::Nested(Box::new(ConditionalDecisionNode {
                condition: Condition::Readable,
                then_decision: ConditionalBranch::Static(Decision::Ask),
                else_decision: ConditionalBranch::Static(Decision::Deny),
            })),
        }));
        assert_eq!(
            resolve_conditional(&d, "/tmp/file.txt", &fa, &cwd()).unwrap(),
            Decision::Allow
        );
        assert_eq!(
            resolve_conditional(&d, "/etc/config.txt", &fa, &cwd()).unwrap(),
            Decision::Ask
        );
        assert_eq!(
            resolve_conditional(&d, "/etc/.secret", &fa, &cwd()).unwrap(),
            Decision::Deny
        );
    }

    // --- read_for_check ---
    //
    // Pure-CPU branches only. The I/O paths (file present, oversized, not
    // UTF-8, generic I/O error) are covered end-to-end by integration tests
    // that run the real binary against a real `checkFile` rule.

    #[test]
    fn read_for_check_blocked_path_no_io() {
        // path matches read denylist (!**/*.secret*); helper must NOT touch
        // the filesystem and must return Unreadable.
        let fa = test_file_access();
        let result = read_for_check("/tmp/some.secret", &fa, &cwd(), 1024).unwrap();
        match result {
            FileCheckRead::Unreadable(msg) => assert!(msg.contains("not readable")),
            FileCheckRead::Contents(_) => panic!("expected Unreadable"),
        }
    }
}
