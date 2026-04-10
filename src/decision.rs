use serde::{Deserialize, Deserializer};

/// Unknown strings cause a deserialization error rather than silently
/// defaulting, since silent fallback could mask typos in security rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Decision {
    Allow,
    Deny,
    Ask,
}

impl Decision {
    pub fn as_str(&self) -> &'static str {
        match self {
            Decision::Allow => "allow",
            Decision::Ask => "ask",
            Decision::Deny => "deny",
        }
    }

    /// Human-readable description for use in reason strings.
    pub fn description(&self) -> &'static str {
        match self {
            Decision::Allow => "is approved",
            Decision::Deny => "is denied",
            Decision::Ask => "requires approval",
        }
    }
}

/// Return the stricter of two decisions.  Deny beats everything, Ask beats
/// Allow.
pub fn stricter(a: Decision, b: Decision) -> Decision {
    match (a, b) {
        (Decision::Deny, _) | (_, Decision::Deny) => Decision::Deny,
        (Decision::Ask, _) | (_, Decision::Ask) => Decision::Ask,
        _ => Decision::Allow,
    }
}

/// Merge multiple decisions: any Deny → Deny, else any Ask → Ask, else Allow.
/// Empty defaults to Ask.
pub fn merge(decisions: &[Decision]) -> Decision {
    decisions
        .iter()
        .copied()
        .reduce(stricter)
        .unwrap_or(Decision::Ask)
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Condition {
    Readable,
    Writable,
}

impl Condition {
    /// Human-readable description for use in reason strings.
    pub fn description(&self) -> &'static str {
        match self {
            Condition::Readable => "readable",
            Condition::Writable => "writable",
        }
    }
}

/// The value inside a `then` or `else` branch of a conditional.  Accepts a
/// bare decision string (`"allow"`) or a nested conditional object
/// (`{ "if": ..., "then": ..., "else": ... }`), but not the
/// `{ "decision": ... }` wrapper that is only valid at the `DecisionNode` level.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum ConditionalBranch {
    Static(Decision),
    Nested(Box<ConditionalDecisionNode>),
}

#[derive(Debug, Clone, Deserialize)]
pub struct ConditionalDecisionNode {
    #[serde(rename = "if")]
    pub condition: Condition,
    #[serde(rename = "then")]
    pub then_decision: ConditionalBranch,
    #[serde(rename = "else")]
    pub else_decision: ConditionalBranch,
}

#[derive(Debug, Clone)]
pub enum DecisionNode {
    Static(Decision),
    Conditional(Box<ConditionalDecisionNode>),
}

#[derive(Debug, Clone)]
pub struct StaticDecisionNode(pub Decision);

#[derive(Deserialize)]
#[serde(untagged)]
enum StaticDecisionNodeRaw {
    String(Decision),
    Object { decision: Decision },
}

impl<'de> Deserialize<'de> for StaticDecisionNode {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        match StaticDecisionNodeRaw::deserialize(d)? {
            StaticDecisionNodeRaw::String(d) => Ok(StaticDecisionNode(d)),
            StaticDecisionNodeRaw::Object { decision } => Ok(StaticDecisionNode(decision)),
        }
    }
}

// Private helpers that express the valid JSON shapes for a DecisionNode:
//   - a bare string:            "allow"
//   - a bare conditional:       { "if": ..., "then": ..., "else": ... }
//   - a decision-keyed object:  { "decision": "allow" }
//                               { "decision": { "if": ..., "then": ..., "else": ... } }

#[derive(Deserialize)]
#[serde(untagged)]
enum DecisionNodeRaw {
    String(Decision),
    BareConditional(ConditionalDecisionNode),
    Object { decision: DecisionNodeValue },
}

#[derive(Deserialize)]
#[serde(untagged)]
enum DecisionNodeValue {
    Static(Decision),
    Conditional(ConditionalDecisionNode),
}

impl<'de> Deserialize<'de> for DecisionNode {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        match DecisionNodeRaw::deserialize(d)? {
            DecisionNodeRaw::String(d)
            | DecisionNodeRaw::Object {
                decision: DecisionNodeValue::Static(d),
            } => Ok(DecisionNode::Static(d)),
            DecisionNodeRaw::BareConditional(c)
            | DecisionNodeRaw::Object {
                decision: DecisionNodeValue::Conditional(c),
            } => Ok(DecisionNode::Conditional(Box::new(c))),
        }
    }
}

/// Result of evaluating a command or node in the decision tree.
pub enum EvalResult {
    /// A fully resolved decision.
    Decided { decision: Decision, reason: String },
    /// Some positional args need filesystem-level path checking before a final
    /// decision can be made. `base_decision` is the merged result of all
    /// non-path decisions collected so far.
    CheckPaths {
        base_decision: Decision,
        path_checks: Vec<PathCheck>,
        reason: String,
    },
}

/// A path that needs conditional resolution against the filesystem.
pub struct PathCheck {
    pub path: String,
    pub decision: DecisionNode,
    /// When true, the resolved decision takes priority over non-forced decisions,
    /// mirroring how forced flag/option judgments work in `merge_judgments`.
    pub force: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stricter_picks_deny() {
        assert_eq!(stricter(Decision::Deny, Decision::Allow), Decision::Deny);
        assert_eq!(stricter(Decision::Allow, Decision::Deny), Decision::Deny);
        assert_eq!(stricter(Decision::Deny, Decision::Ask), Decision::Deny);
    }

    #[test]
    fn stricter_picks_ask_over_allow() {
        assert_eq!(stricter(Decision::Ask, Decision::Allow), Decision::Ask);
        assert_eq!(stricter(Decision::Allow, Decision::Ask), Decision::Ask);
    }

    #[test]
    fn stricter_allow_both() {
        assert_eq!(stricter(Decision::Allow, Decision::Allow), Decision::Allow);
    }

    #[test]
    fn merge_empty_defaults_to_ask() {
        assert_eq!(merge(&[]), Decision::Ask);
    }

    #[test]
    fn merge_single_allow() {
        assert_eq!(merge(&[Decision::Allow]), Decision::Allow);
    }

    #[test]
    fn merge_allow_and_deny_yields_deny() {
        assert_eq!(merge(&[Decision::Allow, Decision::Deny]), Decision::Deny);
    }

    #[test]
    fn merge_allow_and_ask_yields_ask() {
        assert_eq!(merge(&[Decision::Allow, Decision::Ask]), Decision::Ask);
    }

    #[test]
    fn merge_all_three_yields_deny() {
        assert_eq!(
            merge(&[Decision::Allow, Decision::Ask, Decision::Deny]),
            Decision::Deny
        );
    }

    // --- DecisionNode deserialization (custom impl — not trivial serde) ---

    fn de_node(s: &str) -> DecisionNode {
        serde_json::from_str(s).expect("deserialization failed")
    }

    #[test]
    fn decision_node_bare_string() {
        assert!(matches!(
            de_node(r#""allow""#),
            DecisionNode::Static(Decision::Allow)
        ));
    }

    #[test]
    fn decision_node_object_static() {
        assert!(matches!(
            de_node(r#"{ "decision": "deny" }"#),
            DecisionNode::Static(Decision::Deny)
        ));
    }

    #[test]
    fn decision_node_object_conditional() {
        let node =
            de_node(r#"{ "decision": { "if": "readable", "then": "allow", "else": "deny" } }"#);
        assert!(matches!(node, DecisionNode::Conditional(_)));
    }

    #[test]
    fn decision_node_bare_conditional() {
        let node = de_node(r#"{ "if": "readable", "then": "allow", "else": "deny" }"#);
        assert!(matches!(node, DecisionNode::Conditional(_)));
    }

    #[test]
    fn decision_node_nested_conditional() {
        let node = de_node(
            r#"{
            "decision": {
                "if": "writable",
                "then": "allow",
                "else": { "if": "readable", "then": "ask", "else": "deny" }
            }
        }"#,
        );
        let DecisionNode::Conditional(outer) = node else {
            panic!("expected Conditional")
        };
        assert!(matches!(outer.condition, Condition::Writable));
        assert!(matches!(
            outer.then_decision,
            ConditionalBranch::Static(Decision::Allow)
        ));
        let ConditionalBranch::Nested(inner) = &outer.else_decision else {
            panic!("expected Nested")
        };
        assert!(matches!(inner.condition, Condition::Readable));
        assert!(matches!(
            inner.then_decision,
            ConditionalBranch::Static(Decision::Ask)
        ));
        assert!(matches!(
            inner.else_decision,
            ConditionalBranch::Static(Decision::Deny)
        ));
    }
}
