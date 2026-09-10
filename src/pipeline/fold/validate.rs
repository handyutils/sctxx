//! S3 validation — the anti-hallucination gates (spec §8.4).
//!
//! Nothing a model says changes state until it passes here. The gates check
//! what Rust can check: that ids exist, that provenance lies inside the chunk
//! the model was shown, that a constraint quote really appears in a human
//! message, and that lengths and per-kind limits hold. Rejected ops are
//! reported back once in a repair turn and otherwise discarded.

use super::ops::{Confidence, EvtRange, ItemKind, NewItem, Op};
use super::state::FoldState;
use crate::pipeline::ledgers::Ledgers;
use crate::vendor::codex::secrets::{RedactMode, redact};

/// Length limits, in words (spec §8.4).
const MAX_TEXT_WORDS: usize = 60;
const MAX_WHY_WORDS: usize = 40;
const MAX_QUOTE_WORDS: usize = 50;

/// What the validator needs to know about the chunk being folded.
#[derive(Debug)]
pub struct Context<'a> {
    /// Event range the model was shown; provenance must lie inside it.
    pub chunk_range: EvtRange,
    /// Human message text on the active branch, for verbatim quote checking.
    pub human_text: &'a [String],
    /// Event indices that actually appear in the masked view of this chunk.
    pub visible_events: &'a [u32],
    pub ledgers: &'a Ledgers,
    pub redact: RedactMode,
}

/// Why an op was rejected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rejection {
    pub op_index: usize,
    pub op_name: &'static str,
    pub reason: String,
}

impl std::fmt::Display for Rejection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "op #{} ({}): {}",
            self.op_index, self.op_name, self.reason
        )
    }
}

/// Validate a batch against the current state. Returns the accepted ops (with
/// low-confidence downgrades already applied) and the rejections.
pub fn validate(state: &FoldState, ops: &[Op], context: &Context<'_>) -> (Vec<Op>, Vec<Rejection>) {
    let mut accepted: Vec<Op> = Vec::new();
    let mut rejected: Vec<Rejection> = Vec::new();
    // Track per-kind additions within this batch so a single batch cannot blow
    // past a limit that the state alone still satisfies.
    let mut added_of_kind: std::collections::BTreeMap<ItemKind, usize> = Default::default();

    for (index, op) in ops.iter().enumerate() {
        let mut op = op.clone();
        match check(state, &mut op, context, &mut added_of_kind) {
            Ok(()) => accepted.push(op),
            Err(reason) => {
                rejected.push(Rejection {
                    op_index: index,
                    op_name: op.name(),
                    reason,
                });
            }
        }
    }
    (accepted, rejected)
}

fn check(
    state: &FoldState,
    op: &mut Op,
    context: &Context<'_>,
    added_of_kind: &mut std::collections::BTreeMap<ItemKind, usize>,
) -> Result<(), String> {
    // 1. Referenced ids must exist and be active.
    for id in op.target_ids() {
        match state.get(id) {
            None => return Err(format!("unknown item id `{id}`")),
            Some(item) if !item.status.is_active() => {
                return Err(format!("item `{id}` is not active"));
            }
            Some(_) => {}
        }
    }

    // 2. A dropped constraint would silently lose a user rule.
    if let Op::Drop { id, .. } = op
        && state
            .get(id)
            .is_some_and(|item| item.kind == ItemKind::Constraint)
    {
        return Err(
            "a constraint can only be superseded by a later user statement, never dropped".into(),
        );
    }

    // 3. Resolve/confirm event indices must lie in the chunk.
    if let Op::Resolve { evt, .. } | Op::Confirm { evt, .. } = op
        && !context.chunk_range.contains(*evt)
    {
        return Err(format!(
            "evt {evt} is outside this chunk ({})",
            context.chunk_range
        ));
    }

    // 4. Added sources on an update must lie in the chunk.
    if let Op::Update { add_sources, .. } = op {
        for range in add_sources.iter() {
            if !range.within(&context.chunk_range) {
                return Err(format!(
                    "source {range} is outside this chunk ({})",
                    context.chunk_range
                ));
            }
        }
    }

    // 5. New items carry the heaviest checks.
    if let Some(item) = new_item_mut(op) {
        check_new_item(item, context)?;
        let kind = item.kind;
        // Supersede and merge replace rather than add, so they never grow the
        // active set for their kind. Kinds limited to one (goal, current step)
        // are replaced automatically on apply, so adding one is legitimate.
        if let Some(limit) = kind.max_active().filter(|limit| *limit > 1)
            && matches!(op, Op::Add { .. })
        {
            let existing = state.active_of(kind).len();
            let pending = added_of_kind.entry(kind).or_insert(0);
            if existing + *pending >= limit {
                return Err(format!(
                    "at most {limit} active {kind:?} items; use supersede or resolve instead"
                ));
            }
            *pending += 1;
        }
    }

    Ok(())
}

fn new_item_mut(op: &mut Op) -> Option<&mut NewItem> {
    match op {
        Op::Add { item } => Some(item),
        Op::Supersede { replacement, .. } => Some(replacement),
        Op::Merge { merged, .. } => Some(merged),
        _ => None,
    }
}

fn check_new_item(item: &mut NewItem, context: &Context<'_>) -> Result<(), String> {
    // Secrets are re-redacted on the way in: a model can echo something the
    // masking missed.
    item.text = redact(item.text.trim(), context.redact);
    if let Some(why) = &mut item.why {
        *why = redact(why.trim(), context.redact);
    }
    if let Some(quote) = &mut item.quote {
        *quote = redact(quote.trim(), context.redact);
    }

    if item.text.is_empty() {
        return Err("empty text".into());
    }
    if word_count(&item.text) > MAX_TEXT_WORDS {
        return Err(format!(
            "text is {} words; the limit is {MAX_TEXT_WORDS}",
            word_count(&item.text)
        ));
    }
    if let Some(why) = &item.why
        && word_count(why) > MAX_WHY_WORDS
    {
        return Err(format!(
            "why is {} words; the limit is {MAX_WHY_WORDS}",
            word_count(why)
        ));
    }

    if item.sources.is_empty() {
        return Err("no provenance: every item must cite at least one event range".into());
    }
    for range in &item.sources {
        if !range.within(&context.chunk_range) {
            return Err(format!(
                "source {range} is outside this chunk ({})",
                context.chunk_range
            ));
        }
    }
    // At least one cited event must actually be visible in the masked view,
    // so an item cannot cite a range the model never saw.
    let cited_something_visible = item.sources.iter().any(|range| {
        context
            .visible_events
            .iter()
            .any(|evt| range.contains(*evt))
    });
    if !cited_something_visible {
        return Err("none of the cited events appear in this chunk's masked rows".into());
    }

    if item.kind == ItemKind::Constraint {
        let Some(quote) = item.quote.clone() else {
            return Err("a constraint must carry the user's verbatim words in `quote`".into());
        };
        if word_count(&quote) > MAX_QUOTE_WORDS {
            return Err(format!(
                "quote is {} words; the limit is {MAX_QUOTE_WORDS}",
                word_count(&quote)
            ));
        }
        if !quote_appears(&quote, context.human_text) {
            return Err(format!(
                "quote \"{}\" does not appear verbatim in any human message",
                truncate_for_message(&quote)
            ));
        }
    }

    // A path-like token that no ledger and no row mentions is suspicious but
    // not necessarily wrong; lower the confidence rather than reject.
    if mentions_unknown_path(&item.text, context) {
        item.confidence = Confidence::Low;
    }
    Ok(())
}

fn word_count(text: &str) -> usize {
    text.split_whitespace().count()
}

fn truncate_for_message(text: &str) -> String {
    crate::vendor::codex::truncate::truncate_middle_bytes(text, 60)
}

/// Compare after Unicode-ish normalization: case fold, collapse whitespace,
/// and drop surrounding punctuation, so a quote survives reformatting but
/// cannot be invented.
fn normalize_for_quote(text: &str) -> String {
    text.split_whitespace()
        .map(|word| {
            word.trim_matches(|c: char| !c.is_alphanumeric())
                .to_lowercase()
        })
        .filter(|word| !word.is_empty())
        .collect::<Vec<String>>()
        .join(" ")
}

fn quote_appears(quote: &str, human_text: &[String]) -> bool {
    let needle = normalize_for_quote(quote);
    if needle.is_empty() {
        return false;
    }
    human_text
        .iter()
        .any(|text| normalize_for_quote(text).contains(&needle))
}

fn mentions_unknown_path(text: &str, context: &Context<'_>) -> bool {
    let candidates: Vec<&str> = text
        .split_whitespace()
        .map(|word| word.trim_matches(|c: char| "`'\",;:()[]".contains(c)))
        .filter(|word| word.contains('/') && word.contains('.') && !word.contains("://"))
        .collect();
    if candidates.is_empty() {
        return false;
    }
    candidates.iter().any(|candidate| {
        !context
            .ledgers
            .files
            .iter()
            .any(|file| file.path.contains(candidate))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context<'a>(human: &'a [String], ledgers: &'a Ledgers) -> Context<'a> {
        Context {
            chunk_range: EvtRange::new(0, 100),
            human_text: human,
            visible_events: &[0, 10, 20, 50, 100],
            ledgers,
            redact: RedactMode::Default,
        }
    }

    fn item(kind: ItemKind, text: &str) -> NewItem {
        NewItem {
            kind,
            text: text.into(),
            why: None,
            quote: None,
            rejected: vec![],
            sources: vec![EvtRange::new(10, 20)],
            confidence: Confidence::High,
        }
    }

    #[test]
    fn a_well_formed_add_is_accepted() {
        let ledgers = Ledgers::default();
        let human = vec![];
        let (accepted, rejected) = validate(
            &FoldState::new(),
            &[Op::Add {
                item: item(ItemKind::Decision, "use child processes"),
            }],
            &context(&human, &ledgers),
        );
        assert_eq!(accepted.len(), 1);
        assert!(rejected.is_empty(), "{rejected:?}");
    }

    #[test]
    fn provenance_outside_the_chunk_is_rejected() {
        let ledgers = Ledgers::default();
        let human = vec![];
        let mut new = item(ItemKind::Decision, "x");
        new.sources = vec![EvtRange::new(500, 600)];
        let (accepted, rejected) = validate(
            &FoldState::new(),
            &[Op::Add { item: new }],
            &context(&human, &ledgers),
        );
        assert!(accepted.is_empty());
        assert!(
            rejected[0].reason.contains("outside this chunk"),
            "{:?}",
            rejected[0]
        );
    }

    #[test]
    fn an_item_citing_only_invisible_events_is_rejected() {
        let ledgers = Ledgers::default();
        let human = vec![];
        let mut new = item(ItemKind::Decision, "x");
        new.sources = vec![EvtRange::new(30, 31)];
        let (_, rejected) = validate(
            &FoldState::new(),
            &[Op::Add { item: new }],
            &context(&human, &ledgers),
        );
        assert!(
            rejected[0].reason.contains("masked rows"),
            "{:?}",
            rejected[0]
        );
    }

    #[test]
    fn an_item_without_provenance_is_rejected() {
        let ledgers = Ledgers::default();
        let human = vec![];
        let mut new = item(ItemKind::Goal, "ship it");
        new.sources = vec![];
        let (_, rejected) = validate(
            &FoldState::new(),
            &[Op::Add { item: new }],
            &context(&human, &ledgers),
        );
        assert!(
            rejected[0].reason.contains("provenance"),
            "{:?}",
            rejected[0]
        );
    }

    #[test]
    fn a_constraint_needs_a_quote_that_really_appears() {
        let ledgers = Ledgers::default();
        let human = vec!["[user] never auto-install extensions without asking me".to_string()];
        let context = context(&human, &ledgers);

        let mut without = item(ItemKind::Constraint, "do not auto-install");
        without.quote = None;
        let (_, rejected) = validate(&FoldState::new(), &[Op::Add { item: without }], &context);
        assert!(rejected[0].reason.contains("verbatim"), "{:?}", rejected[0]);

        let mut invented = item(ItemKind::Constraint, "do not auto-install");
        invented.quote = Some("always install everything immediately".into());
        let (_, rejected) = validate(&FoldState::new(), &[Op::Add { item: invented }], &context);
        assert!(
            rejected[0].reason.contains("does not appear"),
            "{:?}",
            rejected[0]
        );

        let mut real = item(ItemKind::Constraint, "do not auto-install");
        real.quote = Some("Never auto-install extensions, without asking me".into());
        let (accepted, rejected) = validate(&FoldState::new(), &[Op::Add { item: real }], &context);
        assert_eq!(accepted.len(), 1, "{rejected:?}");
    }

    #[test]
    fn an_over_long_item_is_rejected() {
        let ledgers = Ledgers::default();
        let human = vec![];
        let long = "word ".repeat(80);
        let (_, rejected) = validate(
            &FoldState::new(),
            &[Op::Add {
                item: item(ItemKind::Decision, &long),
            }],
            &context(&human, &ledgers),
        );
        assert!(
            rejected[0].reason.contains("the limit is 60"),
            "{:?}",
            rejected[0]
        );
    }

    #[test]
    fn ops_against_unknown_or_inactive_items_are_rejected() {
        let ledgers = Ledgers::default();
        let human = vec![];
        let mut state = FoldState::new();
        state.apply(
            "c0",
            &Op::Add {
                item: item(ItemKind::OpenThread, "a"),
            },
        );
        state.apply(
            "c0",
            &Op::Resolve {
                id: "O1".into(),
                evt: 1,
            },
        );

        let (_, rejected) = validate(
            &state,
            &[
                Op::Resolve {
                    id: "O9".into(),
                    evt: 10,
                },
                Op::Update {
                    id: "O1".into(),
                    text: Some("b".into()),
                    why: None,
                    add_sources: vec![],
                },
            ],
            &context(&human, &ledgers),
        );
        assert_eq!(rejected.len(), 2);
        assert!(rejected[0].reason.contains("unknown item id"));
        assert!(rejected[1].reason.contains("not active"));
    }

    #[test]
    fn a_constraint_can_never_be_dropped() {
        let ledgers = Ledgers::default();
        let human = vec!["[user] keep the manifest json".to_string()];
        let mut state = FoldState::new();
        let mut constraint = item(ItemKind::Constraint, "manifest stays JSON");
        constraint.quote = Some("keep the manifest json".into());
        state.apply("c0", &Op::Add { item: constraint });

        let (accepted, rejected) = validate(
            &state,
            &[Op::Drop {
                id: "C1".into(),
                reason: "noise".into(),
            }],
            &context(&human, &ledgers),
        );
        assert!(accepted.is_empty());
        assert!(
            rejected[0].reason.contains("never dropped"),
            "{:?}",
            rejected[0]
        );
    }

    #[test]
    fn a_fourth_next_action_in_one_batch_is_rejected() {
        let ledgers = Ledgers::default();
        let human = vec![];
        let ops: Vec<Op> = (0..4)
            .map(|i| Op::Add {
                item: item(ItemKind::NextAction, &format!("step {i}")),
            })
            .collect();
        let (accepted, rejected) = validate(&FoldState::new(), &ops, &context(&human, &ledgers));
        assert_eq!(accepted.len(), 3);
        assert_eq!(rejected.len(), 1);
        assert!(
            rejected[0].reason.contains("supersede"),
            "{:?}",
            rejected[0]
        );
    }

    #[test]
    fn a_path_no_ledger_knows_lowers_confidence_instead_of_rejecting() {
        let ledgers = Ledgers::default();
        let human = vec![];
        let (accepted, rejected) = validate(
            &FoldState::new(),
            &[Op::Add {
                item: item(ItemKind::EnvFact, "config lives in src/never/seen.ts"),
            }],
            &context(&human, &ledgers),
        );
        assert!(rejected.is_empty(), "{rejected:?}");
        assert_eq!(
            accepted[0].new_item().map(|item| item.confidence),
            Some(Confidence::Low)
        );
    }

    #[test]
    fn secrets_in_model_output_are_redacted_on_the_way_in() {
        let ledgers = Ledgers::default();
        let human = vec![];
        let (accepted, _) = validate(
            &FoldState::new(),
            &[Op::Add {
                item: item(
                    ItemKind::EnvFact,
                    "key is ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789",
                ),
            }],
            &context(&human, &ledgers),
        );
        let text = accepted[0]
            .new_item()
            .map(|item| item.text.clone())
            .unwrap_or_default();
        assert!(text.contains("[REDACTED_SECRET]"), "{text}");
    }
}
