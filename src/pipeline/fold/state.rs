//! Fold state and apply semantics (spec §8.1, §8.3).
//!
//! Applying an op is deterministic Rust: ids are assigned here, supersede
//! chains are recorded here, and nothing is ever deleted — a dropped or
//! superseded item stays in `state.json` with its reason, so a reviewer can
//! see what the fold decided and why.

use super::ops::{Confidence, EvtRange, ItemKind, NewItem, Op};
use crate::ir::EventIdx;
use serde::{Deserialize, Serialize};

/// Whether an item is still live.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
#[non_exhaustive]
pub enum ItemStatus {
    Active,
    Superseded { by: String },
    Resolved { evt: EventIdx },
    Dropped { reason: String },
}

impl ItemStatus {
    pub fn is_active(&self) -> bool {
        matches!(self, ItemStatus::Active)
    }
}

/// The result of reconciling an item against the current repository (S5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Verification {
    #[default]
    Unchecked,
    Verified,
    Stale,
    Contradicted,
}

impl Verification {
    pub fn label(self) -> &'static str {
        match self {
            Verification::Unchecked => "unchecked",
            Verification::Verified => "verified",
            Verification::Stale => "stale",
            Verification::Contradicted => "contradicted",
        }
    }
}

/// One piece of carried-forward knowledge.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Item {
    pub id: String,
    pub kind: ItemKind,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub why: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quote: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub rejected: Vec<String>,
    pub sources: Vec<EvtRange>,
    pub status: ItemStatus,
    pub confidence: Confidence,
    pub verified: Verification,
    pub last_confirmed: EventIdx,
}

impl Item {
    /// Provenance rendered for the artifact, e.g. `[evt 12–19]`.
    pub fn provenance(&self) -> String {
        if self.sources.is_empty() {
            return String::new();
        }
        let parts: Vec<String> = self.sources.iter().map(|range| range.to_string()).collect();
        format!("[{}]", parts.join(", "))
    }

    /// The widest range this item cites, for `sctxx expand` hints.
    pub fn span(&self) -> Option<EvtRange> {
        let start = self.sources.iter().map(|range| range.start).min()?;
        let end = self.sources.iter().map(|range| range.end).max()?;
        Some(EvtRange::new(start, end))
    }
}

/// An op that was accepted, kept as an audit trail.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppliedOp {
    pub chunk_id: String,
    pub op: Op,
    /// Ids the op created, if any.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub created: Vec<String>,
}

/// An op that was rejected, kept so a reviewer sees what the model got wrong.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RejectedOp {
    pub chunk_id: String,
    pub op_name: String,
    pub reason: String,
}

/// The evolving handoff state.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct FoldState {
    /// Schema version (`state.v1`).
    pub version: u32,
    pub items: Vec<Item>,
    pub ops_log: Vec<AppliedOp>,
    pub rejected: Vec<RejectedOp>,
    pub processed: Vec<String>,
    /// Next sequence number per item-kind prefix.
    pub next_seq: std::collections::BTreeMap<char, u32>,
}

impl FoldState {
    pub fn new() -> Self {
        Self {
            version: 1,
            ..Default::default()
        }
    }

    /// Active items in rendering priority order, then by id.
    pub fn active(&self) -> Vec<&Item> {
        let mut items: Vec<&Item> = self
            .items
            .iter()
            .filter(|item| item.status.is_active())
            .collect();
        items.sort_by_key(|item| {
            let priority = ItemKind::PRIORITY
                .iter()
                .position(|kind| *kind == item.kind)
                .unwrap_or(usize::MAX);
            (priority, item.id.clone())
        });
        items
    }

    /// Active items of one kind, oldest first.
    pub fn active_of(&self, kind: ItemKind) -> Vec<&Item> {
        self.items
            .iter()
            .filter(|item| item.status.is_active() && item.kind == kind)
            .collect()
    }

    /// Items that were superseded, for the "later reversed" appendix.
    pub fn superseded(&self) -> Vec<&Item> {
        self.items
            .iter()
            .filter(|item| matches!(item.status, ItemStatus::Superseded { .. }))
            .collect()
    }

    pub fn get(&self, id: &str) -> Option<&Item> {
        self.items.iter().find(|item| item.id == id)
    }

    fn get_mut(&mut self, id: &str) -> Option<&mut Item> {
        self.items.iter_mut().find(|item| item.id == id)
    }

    fn next_id(&mut self, kind: ItemKind) -> String {
        let prefix = kind.prefix();
        let seq = self.next_seq.entry(prefix).or_insert(0);
        *seq += 1;
        format!("{prefix}{seq}")
    }

    fn insert(&mut self, new: &NewItem) -> String {
        let id = self.next_id(new.kind);
        let last_confirmed = new.sources.iter().map(|range| range.end).max().unwrap_or(0);
        self.items.push(Item {
            id: id.clone(),
            kind: new.kind,
            text: new.text.clone(),
            why: new.why.clone(),
            quote: new.quote.clone(),
            rejected: new.rejected.clone(),
            sources: new.sources.clone(),
            status: ItemStatus::Active,
            confidence: new.confidence,
            verified: Verification::Unchecked,
            last_confirmed,
        });
        id
    }

    /// Apply one already-validated op. Returns the ids it created.
    ///
    /// Validation is a separate step ([`super::validate`]) so a rejected op can
    /// be reported back to the model without ever touching state.
    pub fn apply(&mut self, chunk_id: &str, op: &Op) -> Vec<String> {
        let mut created = Vec::new();
        match op {
            Op::Add { item } => {
                let id = self.insert(item);
                // Only one current step, goal, or three next actions may be
                // active; a new one supersedes the oldest instead of piling up.
                self.enforce_active_limit(item.kind, &id);
                created.push(id);
            }
            Op::Update {
                id,
                text,
                why,
                add_sources,
            } => {
                if let Some(item) = self.get_mut(id) {
                    if let Some(text) = text {
                        item.text = text.clone();
                    }
                    if let Some(why) = why {
                        item.why = Some(why.clone());
                    }
                    for range in add_sources {
                        if !item.sources.contains(range) {
                            item.sources.push(*range);
                        }
                    }
                    item.last_confirmed = item
                        .sources
                        .iter()
                        .map(|range| range.end)
                        .max()
                        .unwrap_or(item.last_confirmed);
                }
            }
            Op::Supersede {
                id, replacement, ..
            } => {
                let new_id = self.insert(replacement);
                if let Some(item) = self.get_mut(id) {
                    item.status = ItemStatus::Superseded { by: new_id.clone() };
                }
                created.push(new_id);
            }
            Op::Resolve { id, evt } => {
                if let Some(item) = self.get_mut(id) {
                    item.status = ItemStatus::Resolved { evt: *evt };
                }
            }
            Op::Drop { id, reason } => {
                if let Some(item) = self.get_mut(id) {
                    item.status = ItemStatus::Dropped {
                        reason: reason.clone(),
                    };
                }
            }
            Op::Merge { ids, merged } => {
                let new_id = self.insert(merged);
                for id in ids {
                    if let Some(item) = self.get_mut(id) {
                        item.status = ItemStatus::Superseded { by: new_id.clone() };
                    }
                }
                created.push(new_id);
            }
            Op::Confirm { id, evt } => {
                if let Some(item) = self.get_mut(id) {
                    item.last_confirmed = item.last_confirmed.max(*evt);
                    let range = EvtRange::new(*evt, *evt);
                    if !item.sources.contains(&range) {
                        item.sources.push(range);
                    }
                }
            }
        }
        self.ops_log.push(AppliedOp {
            chunk_id: chunk_id.to_string(),
            op: op.clone(),
            created: created.clone(),
        });
        created
    }

    /// Supersede the oldest active items of `kind` until the limit holds.
    fn enforce_active_limit(&mut self, kind: ItemKind, newest_id: &str) {
        let Some(limit) = kind.max_active() else {
            return;
        };
        loop {
            let active: Vec<String> = self
                .items
                .iter()
                .filter(|item| item.status.is_active() && item.kind == kind)
                .map(|item| item.id.clone())
                .collect();
            if active.len() <= limit {
                return;
            }
            let Some(oldest) = active.iter().find(|id| id.as_str() != newest_id) else {
                return;
            };
            let oldest = oldest.clone();
            if let Some(item) = self.get_mut(&oldest) {
                item.status = ItemStatus::Superseded {
                    by: newest_id.to_string(),
                };
            } else {
                return;
            }
        }
    }

    /// Record a rejected op in the audit trail, so a reviewer can see what the
    /// model got wrong and why it never reached the artifact.
    pub fn reject(&mut self, chunk_id: &str, op_name: &str, reason: impl Into<String>) {
        self.rejected.push(RejectedOp {
            chunk_id: chunk_id.to_string(),
            op_name: op_name.to_string(),
            reason: reason.into(),
        });
    }

    /// Mark a chunk processed.
    pub fn mark_processed(&mut self, chunk_id: &str) {
        if !self.processed.iter().any(|id| id == chunk_id) {
            self.processed.push(chunk_id.to_string());
        }
    }

    /// Compact rendering of the active state, as the fold prompt sees it.
    pub fn render_for_prompt(&self) -> String {
        if self.active().is_empty() {
            return "(empty — this is the first chunk)".to_string();
        }
        let mut out = String::new();
        for item in self.active() {
            out.push_str(&format!(
                "{} [{}] {}",
                item.id,
                item.confidence.label(),
                item.text.trim()
            ));
            if let Some(why) = &item.why {
                out.push_str(&format!(" | why: {}", why.trim()));
            }
            if let Some(quote) = &item.quote {
                out.push_str(&format!(" | quote: \"{}\"", quote.trim()));
            }
            out.push_str(&format!(" {}\n", item.provenance()));
        }
        out
    }

    /// Approximate token size of the rendered active state, for budget control.
    pub fn prompt_tokens(&self) -> usize {
        crate::vendor::codex::truncate::approx_token_count(&self.render_for_prompt())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn new_item(kind: ItemKind, text: &str) -> NewItem {
        NewItem {
            kind,
            text: text.to_string(),
            why: None,
            quote: None,
            rejected: Vec::new(),
            sources: vec![EvtRange::new(1, 2)],
            confidence: Confidence::High,
        }
    }

    #[test]
    fn adding_items_assigns_kind_prefixed_ids() {
        let mut state = FoldState::new();
        let created = state.apply(
            "c0",
            &Op::Add {
                item: new_item(ItemKind::Constraint, "no yaml"),
            },
        );
        assert_eq!(created, vec!["C1".to_string()]);
        let created = state.apply(
            "c0",
            &Op::Add {
                item: new_item(ItemKind::Decision, "use zstd"),
            },
        );
        assert_eq!(created, vec!["D1".to_string()]);
        let created = state.apply(
            "c0",
            &Op::Add {
                item: new_item(ItemKind::Constraint, "no npm"),
            },
        );
        assert_eq!(created, vec!["C2".to_string()]);
    }

    #[test]
    fn a_second_current_step_supersedes_the_first_automatically() {
        let mut state = FoldState::new();
        state.apply(
            "c0",
            &Op::Add {
                item: new_item(ItemKind::CurrentStep, "wiring tier 3"),
            },
        );
        state.apply(
            "c1",
            &Op::Add {
                item: new_item(ItemKind::CurrentStep, "fixing spawn"),
            },
        );
        let active = state.active_of(ItemKind::CurrentStep);
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].text, "fixing spawn");
        assert!(matches!(
            state.get("S1").map(|item| &item.status),
            Some(ItemStatus::Superseded { .. })
        ));
    }

    #[test]
    fn superseded_items_stay_in_state_with_a_pointer_to_their_replacement() {
        let mut state = FoldState::new();
        state.apply(
            "c0",
            &Op::Add {
                item: new_item(ItemKind::Decision, "vm.Module"),
            },
        );
        state.apply(
            "c1",
            &Op::Supersede {
                id: "D1".into(),
                replacement: new_item(ItemKind::Decision, "child processes"),
                reason: Some("leaked handles".into()),
            },
        );
        assert_eq!(state.active_of(ItemKind::Decision).len(), 1);
        assert_eq!(state.superseded().len(), 1);
        assert_eq!(
            state.get("D1").map(|item| item.status.clone()),
            Some(ItemStatus::Superseded { by: "D2".into() })
        );
    }

    #[test]
    fn merge_supersedes_every_input_into_one_item() {
        let mut state = FoldState::new();
        state.apply(
            "c0",
            &Op::Add {
                item: new_item(ItemKind::OpenThread, "a"),
            },
        );
        state.apply(
            "c0",
            &Op::Add {
                item: new_item(ItemKind::OpenThread, "b"),
            },
        );
        state.apply(
            "c1",
            &Op::Merge {
                ids: vec!["O1".into(), "O2".into()],
                merged: new_item(ItemKind::OpenThread, "a and b"),
            },
        );
        assert_eq!(state.active_of(ItemKind::OpenThread).len(), 1);
        assert_eq!(state.superseded().len(), 2);
    }

    #[test]
    fn resolve_and_drop_keep_the_item_and_its_reason() {
        let mut state = FoldState::new();
        state.apply(
            "c0",
            &Op::Add {
                item: new_item(ItemKind::OpenThread, "finish it"),
            },
        );
        state.apply(
            "c0",
            &Op::Add {
                item: new_item(ItemKind::EnvFact, "node 24"),
            },
        );
        state.apply(
            "c1",
            &Op::Resolve {
                id: "O1".into(),
                evt: 42,
            },
        );
        state.apply(
            "c1",
            &Op::Drop {
                id: "F1".into(),
                reason: "irrelevant".into(),
            },
        );
        assert_eq!(
            state.get("O1").map(|i| i.status.clone()),
            Some(ItemStatus::Resolved { evt: 42 })
        );
        assert_eq!(
            state.get("F1").map(|i| i.status.clone()),
            Some(ItemStatus::Dropped {
                reason: "irrelevant".into()
            })
        );
        assert!(state.active().is_empty());
    }

    #[test]
    fn update_and_confirm_extend_provenance_without_duplicating_it() {
        let mut state = FoldState::new();
        state.apply(
            "c0",
            &Op::Add {
                item: new_item(ItemKind::Goal, "ship it"),
            },
        );
        state.apply(
            "c1",
            &Op::Update {
                id: "G1".into(),
                text: Some("ship it fully".into()),
                why: None,
                add_sources: vec![EvtRange::new(5, 9), EvtRange::new(1, 2)],
            },
        );
        state.apply(
            "c2",
            &Op::Confirm {
                id: "G1".into(),
                evt: 9,
            },
        );
        let item = state.get("G1").expect("item");
        assert_eq!(item.text, "ship it fully");
        assert_eq!(item.sources.len(), 3, "{:?}", item.sources);
        assert_eq!(item.last_confirmed, 9);
    }

    #[test]
    fn every_applied_op_is_logged_for_audit() {
        let mut state = FoldState::new();
        state.apply(
            "c0",
            &Op::Add {
                item: new_item(ItemKind::Goal, "ship"),
            },
        );
        state.apply(
            "c0",
            &Op::Resolve {
                id: "G1".into(),
                evt: 2,
            },
        );
        assert_eq!(state.ops_log.len(), 2);
        assert_eq!(state.ops_log[0].created, vec!["G1".to_string()]);
    }

    #[test]
    fn no_more_than_three_next_actions_stay_active() {
        let mut state = FoldState::new();
        for i in 0..5 {
            state.apply(
                "c0",
                &Op::Add {
                    item: new_item(ItemKind::NextAction, &format!("step {i}")),
                },
            );
        }
        assert_eq!(state.active_of(ItemKind::NextAction).len(), 3);
    }
}
