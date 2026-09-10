//! The fold's only output: typed operations (spec §8.2).
//!
//! The fold never rewrites prose. It emits operations against an existing
//! state, which Rust validates and applies. That is what makes the result
//! auditable: every item has an op that created it and a provenance range that
//! justifies it.

use crate::ir::EventIdx;
use serde::{Deserialize, Serialize};

/// An inclusive event range `[start, end]`, serialized as a two-element array.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(from = "[EventIdx; 2]", into = "[EventIdx; 2]")]
pub struct EvtRange {
    pub start: EventIdx,
    pub end: EventIdx,
}

impl EvtRange {
    pub fn new(start: EventIdx, end: EventIdx) -> Self {
        if start <= end {
            Self { start, end }
        } else {
            Self {
                start: end,
                end: start,
            }
        }
    }

    pub fn contains(&self, evt: EventIdx) -> bool {
        (self.start..=self.end).contains(&evt)
    }

    /// True when this range lies entirely inside `outer`.
    pub fn within(&self, outer: &EvtRange) -> bool {
        self.start >= outer.start && self.end <= outer.end
    }
}

impl From<[EventIdx; 2]> for EvtRange {
    fn from(value: [EventIdx; 2]) -> Self {
        EvtRange::new(value[0], value[1])
    }
}

impl From<EvtRange> for [EventIdx; 2] {
    fn from(value: EvtRange) -> Self {
        [value.start, value.end]
    }
}

impl std::fmt::Display for EvtRange {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.start == self.end {
            write!(f, "evt {}", self.start)
        } else {
            write!(f, "evt {}–{}", self.start, self.end)
        }
    }
}

/// The kinds of state item the fold may create (spec §8.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ItemKind {
    Constraint,
    Goal,
    CurrentStep,
    NextAction,
    DeadEnd,
    Decision,
    OpenThread,
    EnvFact,
    Question,
}

impl ItemKind {
    /// Id prefix, so an item id says what it is.
    pub fn prefix(self) -> char {
        match self {
            ItemKind::Goal => 'G',
            ItemKind::Constraint => 'C',
            ItemKind::Decision => 'D',
            ItemKind::DeadEnd => 'X',
            ItemKind::EnvFact => 'F',
            ItemKind::OpenThread => 'O',
            ItemKind::CurrentStep => 'S',
            ItemKind::NextAction => 'N',
            ItemKind::Question => 'Q',
        }
    }

    /// Section heading used when rendering.
    pub fn heading(self) -> &'static str {
        match self {
            ItemKind::Constraint => "Constraints",
            ItemKind::Goal => "Goal",
            ItemKind::CurrentStep => "Current step",
            ItemKind::NextAction => "Next actions",
            ItemKind::DeadEnd => "Dead ends",
            ItemKind::Decision => "Decisions",
            ItemKind::OpenThread => "Open threads",
            ItemKind::EnvFact => "Environment facts",
            ItemKind::Question => "Questions for the user",
        }
    }

    /// Budget and rendering priority, most important first (spec §8.1).
    pub const PRIORITY: [ItemKind; 9] = [
        ItemKind::Constraint,
        ItemKind::Goal,
        ItemKind::CurrentStep,
        ItemKind::NextAction,
        ItemKind::DeadEnd,
        ItemKind::Decision,
        ItemKind::OpenThread,
        ItemKind::EnvFact,
        ItemKind::Question,
    ];

    /// How many of this kind may be active at once, if limited.
    pub fn max_active(self) -> Option<usize> {
        match self {
            ItemKind::CurrentStep => Some(1),
            ItemKind::NextAction => Some(3),
            ItemKind::Goal => Some(1),
            _ => None,
        }
    }
}

/// How sure the model was.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Confidence {
    High,
    #[default]
    Medium,
    Low,
}

impl Confidence {
    pub fn label(self) -> &'static str {
        match self {
            Confidence::High => "high",
            Confidence::Medium => "medium",
            Confidence::Low => "low",
        }
    }
}

/// The payload shared by `add`, `supersede.replacement`, and `merge.merged`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NewItem {
    pub kind: ItemKind,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub why: Option<String>,
    /// Required for a constraint: the user's own words, checked verbatim.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quote: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rejected: Vec<String>,
    pub sources: Vec<EvtRange>,
    #[serde(default)]
    pub confidence: Confidence,
}

/// One operation against the fold state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
#[non_exhaustive]
pub enum Op {
    Add {
        #[serde(flatten)]
        item: NewItem,
    },
    Update {
        id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        text: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        why: Option<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        add_sources: Vec<EvtRange>,
    },
    Supersede {
        id: String,
        replacement: NewItem,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
    Resolve {
        id: String,
        evt: EventIdx,
    },
    Drop {
        id: String,
        reason: String,
    },
    Merge {
        ids: Vec<String>,
        merged: NewItem,
    },
    Confirm {
        id: String,
        evt: EventIdx,
    },
}

impl Op {
    /// The existing item ids this op refers to.
    pub fn target_ids(&self) -> Vec<&str> {
        match self {
            Op::Add { .. } => Vec::new(),
            Op::Update { id, .. }
            | Op::Supersede { id, .. }
            | Op::Resolve { id, .. }
            | Op::Drop { id, .. }
            | Op::Confirm { id, .. } => vec![id.as_str()],
            Op::Merge { ids, .. } => ids.iter().map(String::as_str).collect(),
        }
    }

    /// Op name for diagnostics.
    pub fn name(&self) -> &'static str {
        match self {
            Op::Add { .. } => "add",
            Op::Update { .. } => "update",
            Op::Supersede { .. } => "supersede",
            Op::Resolve { .. } => "resolve",
            Op::Drop { .. } => "drop",
            Op::Merge { .. } => "merge",
            Op::Confirm { .. } => "confirm",
        }
    }

    /// The new item this op introduces, if any.
    pub fn new_item(&self) -> Option<&NewItem> {
        match self {
            Op::Add { item } => Some(item),
            Op::Supersede { replacement, .. } => Some(replacement),
            Op::Merge { merged, .. } => Some(merged),
            _ => None,
        }
    }
}

/// One fold response: the ops for one chunk.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OpBatch {
    pub chunk_id: String,
    #[serde(default)]
    pub ops: Vec<Op>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_batch_of_every_op_round_trips_through_json() {
        let json = r#"{
          "chunk_id": "c0",
          "ops": [
            {"op":"add","kind":"constraint","text":"never push to main",
             "quote":"never push to main","sources":[[10,12]],"confidence":"high"},
            {"op":"update","id":"G1","text":"ship the migration","add_sources":[[13,14]]},
            {"op":"supersede","id":"D1","replacement":{"kind":"decision","text":"use child processes",
             "sources":[[20,21]]},"reason":"reversed later"},
            {"op":"resolve","id":"O1","evt":30},
            {"op":"drop","id":"F1","reason":"irrelevant"},
            {"op":"merge","ids":["O2","O3"],"merged":{"kind":"open_thread","text":"finish tier 3",
             "sources":[[40,41]]}},
            {"op":"confirm","id":"C1","evt":50}
          ]
        }"#;
        let batch: OpBatch = serde_json::from_str(json).expect("parse");
        assert_eq!(batch.ops.len(), 7);
        let reserialized = serde_json::to_string(&batch).expect("serialize");
        let again: OpBatch = serde_json::from_str(&reserialized).expect("reparse");
        assert_eq!(batch, again);
    }

    #[test]
    fn an_unknown_field_is_rejected_because_our_own_inputs_are_strict() {
        let json = r#"{"chunk_id":"c0","ops":[{"op":"resolve","id":"O1","evt":1,"extra":true}]}"#;
        assert!(serde_json::from_str::<OpBatch>(json).is_err());
    }

    #[test]
    fn ranges_normalize_and_serialize_as_pairs() {
        let range: EvtRange = serde_json::from_str("[9, 4]").expect("parse");
        assert_eq!(range, EvtRange::new(4, 9));
        assert_eq!(serde_json::to_string(&range).expect("write"), "[4,9]");
        assert!(range.within(&EvtRange::new(0, 10)));
        assert!(!range.within(&EvtRange::new(5, 10)));
        assert_eq!(range.to_string(), "evt 4–9");
    }

    #[test]
    fn item_ids_are_prefixed_by_kind() {
        assert_eq!(ItemKind::Constraint.prefix(), 'C');
        assert_eq!(ItemKind::DeadEnd.prefix(), 'X');
        assert_eq!(ItemKind::PRIORITY[0], ItemKind::Constraint);
        assert_eq!(ItemKind::CurrentStep.max_active(), Some(1));
        assert_eq!(ItemKind::Decision.max_active(), None);
    }

    #[test]
    fn an_empty_op_list_is_a_valid_no_op_response() {
        let batch: OpBatch = serde_json::from_str(r#"{"chunk_id":"c3","ops":[]}"#).expect("parse");
        assert!(batch.ops.is_empty());
    }
}
