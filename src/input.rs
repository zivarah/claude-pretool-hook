#![allow(dead_code)]

use std::collections::HashMap;

use serde::Deserialize;

// ---------------------------------------------------------------------------
// Common and agent fields
// ---------------------------------------------------------------------------

/// Fields present on every hook event.
#[derive(Debug, Default)]
pub struct CommonInput {
    pub session_id: String,
    pub transcript_path: String,
    pub cwd: String,
    pub permission_mode: Option<String>,
    pub hook_event_name: String,
    pub tool_use_id: Option<String>,
    /// Codex extension identifying the active turn.  Claude Code never sends
    /// it, so its presence is used to detect a Codex caller and select the
    /// output format Codex acts on.
    pub turn_id: Option<String>,
}

impl CommonInput {
    /// Whether the payload originated from OpenAI Codex rather than Claude
    /// Code, detected via the Codex-only `turn_id` field.
    pub fn is_codex(&self) -> bool {
        self.turn_id.is_some()
    }
}

/// Additional fields present when the hook fires inside a subagent.
#[derive(Debug)]
pub struct AgentInput {
    /// Unique identifier for the subagent (present only inside a subagent call).
    pub agent_id: Option<String>,
    /// Agent name (e.g. "Explore"). Present when `--agent` is used or inside a subagent.
    pub agent_type: Option<String>,
}

// ---------------------------------------------------------------------------
// Per-tool input structs
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct BashInput {
    pub command: String,
    pub description: Option<String>,
    pub timeout: Option<u64>,
    pub run_in_background: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct WriteInput {
    pub file_path: String,
    pub content: String,
}

#[derive(Debug, Deserialize)]
pub struct EditInput {
    pub file_path: String,
    pub old_string: String,
    pub new_string: String,
    #[serde(default)]
    pub replace_all: bool,
}

#[derive(Debug, Deserialize)]
pub struct NotebookEditInput {
    pub file_path: String,
}

#[derive(Debug, Deserialize)]
pub struct ReadInput {
    pub file_path: String,
    pub offset: Option<u64>,
    pub limit: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub struct GlobInput {
    pub pattern: String,
    pub path: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct GrepInput {
    pub pattern: String,
    pub path: Option<String>,
    pub glob: Option<String>,
    pub output_mode: Option<String>,
    #[serde(rename = "-i", default)]
    pub case_insensitive: bool,
    #[serde(default)]
    pub multiline: bool,
}

#[derive(Debug, Deserialize)]
pub struct WebFetchInput {
    pub url: String,
    pub prompt: String,
}

#[derive(Debug, Deserialize)]
pub struct WebSearchInput {
    pub query: String,
    pub allowed_domains: Option<Vec<String>>,
    pub blocked_domains: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
pub struct AgentToolInput {
    pub prompt: String,
    pub description: Option<String>,
    pub subagent_type: Option<String>,
    pub model: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct QuestionOption {
    pub label: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Question {
    pub question: String,
    pub header: String,
    pub options: Vec<QuestionOption>,
    #[serde(default)]
    pub multi_select: bool,
}

#[derive(Debug, Deserialize)]
pub struct AskUserQuestionInput {
    pub questions: Vec<Question>,
    /// Maps question text to selected option label(s) when pre-answered.
    pub answers: Option<HashMap<String, String>>,
}

/// Input for Codex's `apply_patch` tool.  The whole change set is carried as a
/// single patch-envelope string; the exact field name varies across Codex
/// versions, so several known spellings are accepted.
#[derive(Debug, Deserialize)]
pub struct ApplyPatchInput {
    #[serde(alias = "patch", alias = "changes", alias = "content", alias = "diff")]
    pub input: String,
}

impl ApplyPatchInput {
    /// Extract the set of file paths the patch writes to by scanning the
    /// envelope's `*** Add File:`, `*** Update File:`, `*** Delete File:`, and
    /// `*** Move to:` header lines.  A rename produces both the original
    /// (`Update File`) and the destination (`Move to`) paths.
    pub fn affected_paths(&self) -> Vec<String> {
        const PREFIXES: [&str; 4] = [
            "*** Add File:",
            "*** Update File:",
            "*** Delete File:",
            "*** Move to:",
        ];
        self.input
            .lines()
            .filter_map(|line| {
                let line = line.trim();
                PREFIXES.iter().find_map(|prefix| {
                    line.strip_prefix(prefix)
                        .map(str::trim)
                        .filter(|p| !p.is_empty())
                        .map(str::to_owned)
                })
            })
            .collect()
    }
}

// ---------------------------------------------------------------------------
// ToolInput enum
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub enum ToolInput {
    Bash(BashInput),
    Write(WriteInput),
    Edit(EditInput),
    NotebookEdit(NotebookEditInput),
    Read(ReadInput),
    Glob(GlobInput),
    Grep(GrepInput),
    WebFetch(WebFetchInput),
    WebSearch(WebSearchInput),
    Agent(AgentToolInput),
    AskUserQuestion(AskUserQuestionInput),
    /// Codex's file-editing tool (no Claude Code equivalent).
    ApplyPatch(ApplyPatchInput),
    /// Forward-compatibility catch-all for tools not listed above.
    Unknown {
        tool_name: String,
        tool_input: serde_json::Value,
    },
}

impl ToolInput {
    pub fn name(&self) -> &str {
        match self {
            ToolInput::Bash(_) => "Bash",
            ToolInput::Write(_) => "Write",
            ToolInput::Edit(_) => "Edit",
            ToolInput::NotebookEdit(_) => "NotebookEdit",
            ToolInput::Read(_) => "Read",
            ToolInput::Glob(_) => "Glob",
            ToolInput::Grep(_) => "Grep",
            ToolInput::WebFetch(_) => "WebFetch",
            ToolInput::WebSearch(_) => "WebSearch",
            ToolInput::Agent(_) => "Agent",
            ToolInput::AskUserQuestion(_) => "AskUserQuestion",
            ToolInput::ApplyPatch(_) => "apply_patch",
            ToolInput::Unknown { tool_name, .. } => tool_name,
        }
    }
}

// ---------------------------------------------------------------------------
// HookInput — custom Deserialize needed because ToolInput's variant depends
// on `tool_name`, a sibling field of `tool_input` in the flat JSON object.
// ---------------------------------------------------------------------------

pub struct HookInput {
    pub common: CommonInput,
    pub agent: Option<AgentInput>,
    pub tool: ToolInput,
}

impl<'de> Deserialize<'de> for HookInput {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        /// Flat view of the hook JSON payload.  All common fields default so
        /// that test fixtures that omit them still parse successfully.
        #[derive(Deserialize)]
        struct Flat {
            #[serde(default)]
            session_id: String,
            #[serde(default)]
            transcript_path: String,
            #[serde(default)]
            cwd: String,
            permission_mode: Option<String>,
            #[serde(default)]
            hook_event_name: String,
            tool_use_id: Option<String>,
            turn_id: Option<String>,
            agent_id: Option<String>,
            agent_type: Option<String>,
            #[serde(default)]
            tool_name: String,
            #[serde(default)]
            tool_input: serde_json::Value,
        }

        let flat = Flat::deserialize(deserializer)?;

        let common = CommonInput {
            session_id: flat.session_id,
            transcript_path: flat.transcript_path,
            cwd: flat.cwd,
            permission_mode: flat.permission_mode,
            hook_event_name: flat.hook_event_name,
            tool_use_id: flat.tool_use_id,
            turn_id: flat.turn_id,
        };

        let agent = if flat.agent_id.is_some() || flat.agent_type.is_some() {
            Some(AgentInput {
                agent_id: flat.agent_id,
                agent_type: flat.agent_type,
            })
        } else {
            None
        };

        let tool =
            parse_tool_input(&flat.tool_name, flat.tool_input).map_err(serde::de::Error::custom)?;

        Ok(HookInput {
            common,
            agent,
            tool,
        })
    }
}

/// Returned when a known tool's `tool_input` cannot be deserialized into
/// the expected struct.  Unknown tool names never produce this error; they
/// fall through to `ToolInput::Unknown` instead.
#[derive(Debug, thiserror::Error)]
#[error("failed to parse input for known tool {tool_name:?}: {source}")]
pub struct ParseToolInputError {
    pub tool_name: String,
    #[source]
    source: serde_json::Error,
}

/// Deserialize `tool_input` into the appropriate `ToolInput` variant based on
/// `tool_name`.  For known tool names, a deserialization failure is returned
/// as an error rather than silently falling back to `Unknown`.  Unrecognized
/// tool names still produce `Unknown` for forward compatibility.
fn parse_tool_input(
    tool_name: &str,
    value: serde_json::Value,
) -> Result<ToolInput, ParseToolInputError> {
    macro_rules! try_parse {
        ($variant:expr) => {
            return serde_json::from_value(value)
                .map($variant)
                .map_err(|e| ParseToolInputError {
                    tool_name: tool_name.to_owned(),
                    source: e,
                })
        };
    }

    match tool_name {
        "Bash" => try_parse!(ToolInput::Bash),
        "Write" => try_parse!(ToolInput::Write),
        "Edit" => try_parse!(ToolInput::Edit),
        "NotebookEdit" => try_parse!(ToolInput::NotebookEdit),
        "Read" => try_parse!(ToolInput::Read),
        "Glob" => try_parse!(ToolInput::Glob),
        "Grep" => try_parse!(ToolInput::Grep),
        "WebFetch" => try_parse!(ToolInput::WebFetch),
        "WebSearch" => try_parse!(ToolInput::WebSearch),
        "Agent" => try_parse!(ToolInput::Agent),
        "AskUserQuestion" => try_parse!(ToolInput::AskUserQuestion),
        "apply_patch" => try_parse!(ToolInput::ApplyPatch),
        _ => {}
    }

    Ok(ToolInput::Unknown {
        tool_name: tool_name.to_owned(),
        tool_input: value,
    })
}
