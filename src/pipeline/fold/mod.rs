//! S3 — the anchored fold (spec §8.5).
//!
//! The fold walks chunks in order. Each call sees the current state, the
//! deterministic ledger slice for its chunk, and the chunk's masked rows, and
//! answers with operations. Because state carries forward and every op is
//! validated against the chunk it came from, the result is *anchored*: it
//! cannot drift into a summary of a summary, and every claim keeps a pointer
//! back into the transcript.

pub mod ops;
pub mod prompt;
pub mod state;
pub mod validate;

use crate::error::Result;
use crate::ir::Session;
use crate::llm::{Backend, CallRole, Request};
use crate::pipeline::ledgers::Ledgers;
use crate::pipeline::mask::Row;
use crate::pipeline::segment::Plan;
use crate::vendor::codex::secrets::RedactMode;
use ops::{EvtRange, OpBatch};
use state::FoldState;

/// Fold configuration.
#[derive(Debug, Clone)]
pub struct FoldOptions {
    /// Optional user intent that biases extraction.
    pub focus: Option<String>,
    /// Run the isolated premap pass when there are more chunks than this.
    pub premap_threshold: usize,
    /// Parallel premap calls.
    pub concurrency: usize,
    /// Warn the model when the rendered state exceeds this many tokens.
    pub state_tokens: usize,
    pub redact: RedactMode,
}

impl Default for FoldOptions {
    fn default() -> Self {
        Self {
            focus: None,
            premap_threshold: 4,
            concurrency: 4,
            state_tokens: 6_000,
            redact: RedactMode::Default,
        }
    }
}

/// What the fold produced, alongside the state itself.
#[derive(Debug, Clone, Default)]
pub struct FoldReport {
    pub calls: usize,
    pub repair_calls: usize,
    pub premap_calls: usize,
    pub accepted_ops: usize,
    pub rejected_ops: usize,
    pub input_tokens: u64,
    pub output_tokens: u64,
    /// Backend failures that did not abort the run.
    pub warnings: Vec<String>,
}

/// Everything the fold needs from the deterministic stages.
#[derive(Debug)]
pub struct FoldInput<'a> {
    pub session: &'a Session,
    pub ledgers: &'a Ledgers,
    pub rows: &'a [Row],
    pub plan: &'a Plan,
}

/// Run premap, the sequential fold, and the final pass.
pub fn run(
    input: &FoldInput<'_>,
    backend: &dyn Backend,
    options: &FoldOptions,
) -> Result<(FoldState, FoldReport)> {
    let mut state = FoldState::new();
    let mut report = FoldReport::default();
    let human_text = human_messages(input.ledgers);
    let session_label = format!("{}:{}", input.session.agent.slug(), input.session.id);
    let focus = options
        .focus
        .as_ref()
        .map(|focus| format!("FOCUS: the next agent wants to: {focus}"))
        .unwrap_or_default();

    // S3a premap: independent per chunk, so it parallelizes. Its output is
    // advisory; the sequential pass still decides what enters the state.
    let premap_notes = if input.plan.chunks.len() > options.premap_threshold {
        premap(input, backend, options, &focus, &mut report)
    } else {
        vec![String::new(); input.plan.chunks.len()]
    };

    // S3b the sequential anchored fold.
    for (index, chunk) in input.plan.chunks.iter().enumerate() {
        let range = EvtRange::new(chunk.evt_start, chunk.evt_end);
        let rows = &input.rows[chunk.rows.clone()];
        let mut chunk_text = crate::pipeline::mask::render(rows);
        if let Some(notes) = premap_notes.get(index).filter(|notes| !notes.is_empty()) {
            chunk_text.push_str("\n# CANDIDATES FROM AN ISOLATED PASS OVER THIS CHUNK\n");
            chunk_text.push_str(notes);
        }

        let fields = prompt::Fields {
            chunk_id: chunk.id.clone(),
            session: session_label.clone(),
            focus: budget_notice(&state, options, &focus),
            state: state.render_for_prompt(),
            ledger_slice: ledger_slice(input.ledgers, range),
            later_index: later_index(input, index),
            chunk: chunk_text,
            range,
            rejections: String::new(),
        };
        let user = prompt::render(prompt::Template::FoldUser, &fields, prompt::OPS_SCHEMA);

        let context = validate::Context {
            chunk_range: range,
            human_text: &human_text,
            visible_events: &visible_events(rows),
            ledgers: input.ledgers,
            redact: options.redact,
        };
        apply_call(
            backend,
            CallRole::Fold,
            &user,
            &chunk.id,
            &context,
            &mut state,
            &mut report,
            &fields,
            true,
        );
        state.mark_processed(&chunk.id);
    }

    // S3c the final pass over the recency tail, which the fold never saw.
    if !input.plan.tail.is_empty() {
        let rows = &input.rows[input.plan.tail.clone()];
        let range = EvtRange::new(
            rows.first().map(|row| row.evt).unwrap_or(0),
            rows.last().map(|row| row.evt).unwrap_or(0),
        );
        let fields = prompt::Fields {
            chunk_id: "final".to_string(),
            session: session_label,
            focus: focus.clone(),
            state: state.render_for_prompt(),
            ledger_slice: last_known_state(input.ledgers),
            later_index: "(nothing — this is the end of the session)".to_string(),
            chunk: crate::pipeline::mask::render(rows),
            range,
            rejections: String::new(),
        };
        let user = prompt::render(prompt::Template::FinalPass, &fields, prompt::OPS_SCHEMA);
        let context = validate::Context {
            chunk_range: range,
            human_text: &human_text,
            visible_events: &visible_events(rows),
            ledgers: input.ledgers,
            redact: options.redact,
        };
        apply_call(
            backend,
            CallRole::FinalPass,
            &user,
            "final",
            &context,
            &mut state,
            &mut report,
            &fields,
            true,
        );
        state.mark_processed("final");
    }

    Ok((state, report))
}

/// One model call, validated, with a single repair turn on rejection.
#[allow(clippy::too_many_arguments)]
fn apply_call(
    backend: &dyn Backend,
    role: CallRole,
    user: &str,
    chunk_id: &str,
    context: &validate::Context<'_>,
    state: &mut FoldState,
    report: &mut FoldReport,
    fields: &prompt::Fields,
    allow_repair: bool,
) {
    let request = Request {
        role,
        system: strip(prompt::FOLD_SYSTEM),
        user: user.to_string(),
        json_schema: serde_json::from_str(prompt::OPS_SCHEMA).ok(),
        max_output_tokens: 4_000,
        temperature: Some(0.0),
    };
    report.calls += 1;
    let response = match backend.complete(&request) {
        Ok(response) => response,
        Err(error) => {
            // A backend failure loses this chunk's semantic pass, not the run:
            // the deterministic parts of the artifact are still correct.
            report.warnings.push(format!("{chunk_id}: {error}"));
            return;
        }
    };
    report.input_tokens += response.input_tokens.unwrap_or(0);
    report.output_tokens += response.output_tokens.unwrap_or(0);

    let batch = match parse_batch(&response.text, chunk_id) {
        Ok(batch) => batch,
        Err(reason) => {
            report
                .warnings
                .push(format!("{chunk_id}: unusable response ({reason})"));
            return;
        }
    };

    let (accepted, rejected) = validate::validate(state, &batch.ops, context);
    for op in &accepted {
        state.apply(chunk_id, op);
    }
    report.accepted_ops += accepted.len();
    report.rejected_ops += rejected.len();

    for rejection in &rejected {
        state.reject(chunk_id, rejection.op_name, rejection.reason.clone());
    }
    if rejected.is_empty() || !allow_repair {
        return;
    }

    // The repair turn: tell the model exactly what was wrong, once.
    let rejections = rejected
        .iter()
        .map(|r| format!("- {r}\n"))
        .collect::<String>();
    let repair_fields = prompt::Fields {
        rejections,
        ..fields.clone()
    };
    let repair_user = prompt::render(prompt::Template::Repair, &repair_fields, prompt::OPS_SCHEMA);
    report.repair_calls += 1;
    apply_call(
        backend,
        role,
        &repair_user,
        chunk_id,
        context,
        state,
        report,
        &repair_fields,
        false,
    );
}

fn parse_batch(text: &str, chunk_id: &str) -> std::result::Result<OpBatch, String> {
    let mut value = crate::llm::repair::parse_object(text)?;
    // A model that forgets `chunk_id` should not lose a whole chunk of work.
    match value.as_object_mut() {
        Some(object) => {
            object
                .entry("chunk_id")
                .or_insert_with(|| serde_json::Value::String(chunk_id.to_string()));
        }
        None => return Err("the response was not a JSON object".to_string()),
    }
    serde_json::from_value(value).map_err(|error| error.to_string())
}

/// S3a: one isolated call per chunk, run on scoped OS threads.
fn premap(
    input: &FoldInput<'_>,
    backend: &dyn Backend,
    options: &FoldOptions,
    focus: &str,
    report: &mut FoldReport,
) -> Vec<String> {
    let chunks = &input.plan.chunks;
    let concurrency = options.concurrency.clamp(1, 16);
    let mut notes = vec![String::new(); chunks.len()];
    let mut warnings: Vec<String> = Vec::new();
    let mut calls = 0usize;

    for (batch_index, batch) in chunks.chunks(concurrency).enumerate() {
        let offset = batch_index * concurrency;
        let results: Vec<(usize, std::result::Result<String, String>)> =
            std::thread::scope(|scope| {
                let handles: Vec<_> = batch
                    .iter()
                    .enumerate()
                    .map(|(index, chunk)| {
                        let range = EvtRange::new(chunk.evt_start, chunk.evt_end);
                        let rows = &input.rows[chunk.rows.clone()];
                        let fields = prompt::Fields {
                            chunk_id: chunk.id.clone(),
                            focus: focus.to_string(),
                            ledger_slice: ledger_slice(input.ledgers, range),
                            chunk: crate::pipeline::mask::render(rows),
                            range,
                            ..Default::default()
                        };
                        let user =
                            prompt::render(prompt::Template::Premap, &fields, prompt::OPS_SCHEMA);
                        scope.spawn(move || {
                            let request = Request {
                                role: CallRole::Premap,
                                system: strip(prompt::FOLD_SYSTEM),
                                user,
                                json_schema: None,
                                max_output_tokens: 2_000,
                                temperature: Some(0.0),
                            };
                            (
                                offset + index,
                                backend
                                    .complete(&request)
                                    .map(|response| response.text)
                                    .map_err(|error| error.to_string()),
                            )
                        })
                    })
                    .collect();
                handles
                    .into_iter()
                    .filter_map(|handle| handle.join().ok())
                    .collect()
            });

        for (index, result) in results {
            calls += 1;
            match result {
                Ok(text) => {
                    if let Some(slot) = notes.get_mut(index) {
                        *slot = summarize_candidates(&text);
                    }
                }
                Err(error) => warnings.push(format!("premap chunk {index}: {error}")),
            }
        }
    }

    report.premap_calls = calls;
    report.calls += calls;
    report.warnings.extend(warnings);
    notes
}

/// Reduce a premap response to the candidate lines the fold call will read.
fn summarize_candidates(text: &str) -> String {
    let Ok(value) = crate::llm::repair::parse_object(text) else {
        return String::new();
    };
    let Some(ops) = value.get("ops").and_then(|ops| ops.as_array()) else {
        return String::new();
    };
    let mut out = String::new();
    for op in ops.iter().take(12) {
        let kind = op
            .get("kind")
            .and_then(|kind| kind.as_str())
            .unwrap_or("item");
        let item_text = op.get("text").and_then(|text| text.as_str()).unwrap_or("");
        if item_text.is_empty() {
            continue;
        }
        let sources = op
            .get("sources")
            .map(|sources| sources.to_string())
            .unwrap_or_else(|| "[]".to_string());
        out.push_str(&format!("- candidate {kind}: {item_text} {sources}\n"));
    }
    out
}

/// Tell the model when the state has outgrown its budget, so it merges and
/// drops instead of the Rust side truncating silently (spec §8.5).
fn budget_notice(state: &FoldState, options: &FoldOptions, focus: &str) -> String {
    let tokens = state.prompt_tokens();
    if tokens <= options.state_tokens {
        return focus.to_string();
    }
    format!(
        "{focus}\nSTATE BUDGET: the state is {} tokens over the {} token budget. Include merge \
         and drop operations for the lowest-value items first. Never drop a constraint.",
        tokens - options.state_tokens,
        options.state_tokens
    )
}

/// Human message text on the active branch, for verbatim quote checking.
fn human_messages(ledgers: &Ledgers) -> Vec<String> {
    ledgers
        .user_messages
        .iter()
        .map(|message| message.text.clone())
        .collect()
}

fn visible_events(rows: &[Row]) -> Vec<u32> {
    rows.iter().map(|row| row.evt).collect()
}

/// The deterministic facts for one chunk's event range.
fn ledger_slice(ledgers: &Ledgers, range: EvtRange) -> String {
    let mut out = String::new();

    let files: Vec<String> = ledgers
        .files
        .iter()
        .filter(|file| file.last_evt >= range.start && file.first_evt <= range.end)
        .map(|file| {
            let mut label = file.path.clone();
            if file.edits > 0 {
                label.push_str(&format!(" (edit×{})", file.edits));
            } else if file.reads > 0 {
                label.push_str(&format!(" (read×{})", file.reads));
            }
            if file.inferred {
                label.push_str(" [inferred]");
            }
            label
        })
        .collect();
    if !files.is_empty() {
        out.push_str(&format!("Files touched: {}\n", files.join(", ")));
    }

    let commands: Vec<String> = ledgers
        .commands
        .iter()
        .filter(|command| range.contains(command.evt))
        .map(|command| {
            format!(
                "`{}` → {} [evt {}]",
                command.normalized,
                command.status(),
                command.evt
            )
        })
        .collect();
    if !commands.is_empty() {
        out.push_str(&format!("Commands: {}\n", commands.join("; ")));
    }

    let errors: Vec<String> = ledgers
        .errors
        .iter()
        .filter(|error| error.last_evt >= range.start && error.first_evt <= range.end)
        .map(|error| {
            format!(
                "{} ×{} — {}",
                error.sig,
                error.occurrences,
                error.example.lines().next().unwrap_or("").trim()
            )
        })
        .collect();
    if !errors.is_empty() {
        out.push_str(&format!("Error signatures: {}\n", errors.join("; ")));
    }

    let dead_ends: Vec<String> = ledgers
        .dead_end_candidates()
        .iter()
        .filter(|error| error.last_evt >= range.start && error.first_evt <= range.end)
        .map(|error| format!("{} (×{}, still unresolved)", error.sig, error.occurrences))
        .collect();
    if !dead_ends.is_empty() {
        out.push_str(&format!(
            "Repeatedly unresolved (dead-end candidates): {}\n",
            dead_ends.join("; ")
        ));
    }

    if out.is_empty() {
        "(nothing recorded for this range)".to_string()
    } else {
        out
    }
}

/// The last-known deterministic state, for the final pass.
fn last_known_state(ledgers: &Ledgers) -> String {
    let mut out = String::new();
    for command in ledgers.last_command_status().iter().take(10) {
        out.push_str(&format!(
            "`{}` → {} [evt {}]\n",
            command.normalized,
            command.status(),
            command.evt
        ));
    }
    let unresolved = ledgers.unresolved_errors();
    if !unresolved.is_empty() {
        out.push_str("Still failing at the end:\n");
        for error in unresolved.iter().take(8) {
            out.push_str(&format!(
                "- {} ×{} — {}\n",
                error.sig,
                error.occurrences,
                error.example.lines().next().unwrap_or("").trim()
            ));
        }
    }
    if let Some(plan) = &ledgers.plan {
        out.push_str("Last published plan:\n");
        for item in &plan.items {
            out.push_str(&format!("- [{}] {}\n", item.status, item.text));
        }
    }
    if out.is_empty() {
        "(nothing recorded)".to_string()
    } else {
        out
    }
}

/// One line per later episode, so the fold does not mark resolved work open.
fn later_index(input: &FoldInput<'_>, current_chunk: usize) -> String {
    let current_episodes: Vec<usize> = input
        .plan
        .chunks
        .get(current_chunk)
        .map(|chunk| chunk.episode_ids.clone())
        .unwrap_or_default();
    let after = current_episodes.iter().max().copied().unwrap_or(0);
    let lines: Vec<String> = input
        .plan
        .episodes
        .iter()
        .filter(|episode| episode.id > after)
        .map(|episode| {
            format!(
                "- e{} (evt {}–{}): {}",
                episode.id, episode.evt_start, episode.evt_end, episode.headline
            )
        })
        .collect();
    if lines.is_empty() {
        "(nothing — this is the last chunk before the recency tail)".to_string()
    } else {
        lines.join("\n")
    }
}

fn strip(body: &str) -> String {
    // The system prompt keeps its rules but not its front matter.
    let after_comment = match body.find("-->") {
        Some(end) => &body[end + 3..],
        None => body,
    };
    let trimmed = after_comment.trim_start();
    let Some(rest) = trimmed.strip_prefix("---") else {
        return trimmed.to_string();
    };
    match rest.find("\n---") {
        Some(end) => rest[end + 4..].trim_start().to_string(),
        None => trimmed.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::mock::Mock;
    use ops::Op;

    #[test]
    fn a_backend_failure_warns_instead_of_aborting_the_run() {
        let backend = Mock::scripted(vec![]);
        let mut state = FoldState::new();
        let mut report = FoldReport::default();
        let ledgers = Ledgers::default();
        let human: Vec<String> = vec![];
        let context = validate::Context {
            chunk_range: EvtRange::new(0, 10),
            human_text: &human,
            visible_events: &[0],
            ledgers: &ledgers,
            redact: RedactMode::Default,
        };
        apply_call(
            &backend,
            CallRole::Fold,
            "prompt",
            "c0",
            &context,
            &mut state,
            &mut report,
            &prompt::Fields::default(),
            true,
        );
        assert!(state.items.is_empty());
        assert_eq!(report.warnings.len(), 1);
        assert!(report.warnings[0].contains("c0"), "{:?}", report.warnings);
    }

    #[test]
    fn a_malformed_response_is_repaired_rather_than_lost() {
        // Fenced JSON with a trailing comma: the repair pass must recover it.
        let backend = Mock::scripted(vec![
            "```json\n{\"chunk_id\":\"c0\",\"ops\":[{\"op\":\"add\",\"kind\":\"goal\",\"text\":\"ship it\",\"sources\":[[0,5]]},]}\n```"
                .to_string(),
        ]);
        let mut state = FoldState::new();
        let mut report = FoldReport::default();
        let ledgers = Ledgers::default();
        let human: Vec<String> = vec![];
        let context = validate::Context {
            chunk_range: EvtRange::new(0, 10),
            human_text: &human,
            visible_events: &[0, 5],
            ledgers: &ledgers,
            redact: RedactMode::Default,
        };
        apply_call(
            &backend,
            CallRole::Fold,
            "prompt",
            "c0",
            &context,
            &mut state,
            &mut report,
            &prompt::Fields::default(),
            true,
        );
        assert_eq!(state.active().len(), 1, "{:?}", report.warnings);
        assert_eq!(report.accepted_ops, 1);
    }

    #[test]
    fn a_rejected_op_triggers_exactly_one_repair_turn() {
        // First response cites a range outside the chunk; the repair fixes it.
        let backend = Mock::scripted(vec![
            r#"{"chunk_id":"c0","ops":[{"op":"add","kind":"goal","text":"ship","sources":[[900,999]]}]}"#.to_string(),
            r#"{"chunk_id":"c0","ops":[{"op":"add","kind":"goal","text":"ship","sources":[[0,5]]}]}"#.to_string(),
            r#"{"chunk_id":"c0","ops":[]}"#.to_string(),
        ]);
        let mut state = FoldState::new();
        let mut report = FoldReport::default();
        let ledgers = Ledgers::default();
        let human: Vec<String> = vec![];
        let context = validate::Context {
            chunk_range: EvtRange::new(0, 10),
            human_text: &human,
            visible_events: &[0, 5],
            ledgers: &ledgers,
            redact: RedactMode::Default,
        };
        apply_call(
            &backend,
            CallRole::Fold,
            "prompt",
            "c0",
            &context,
            &mut state,
            &mut report,
            &prompt::Fields::default(),
            true,
        );
        assert_eq!(report.repair_calls, 1);
        assert_eq!(report.rejected_ops, 1);
        assert_eq!(state.active().len(), 1);
        // The third scripted response must never be consumed: one repair only.
        assert_eq!(backend.calls().len(), 2);
    }

    #[test]
    fn the_state_budget_notice_only_appears_when_over_budget() {
        let options = FoldOptions {
            state_tokens: 0,
            ..FoldOptions::default()
        };
        let mut state = FoldState::new();
        state.apply(
            "c0",
            &Op::Add {
                item: ops::NewItem {
                    kind: ops::ItemKind::Goal,
                    text: "ship it".into(),
                    why: None,
                    quote: None,
                    rejected: vec![],
                    sources: vec![EvtRange::new(0, 1)],
                    confidence: ops::Confidence::High,
                },
            },
        );
        assert!(budget_notice(&state, &options, "").contains("STATE BUDGET"));
        let generous = FoldOptions::default();
        assert!(!budget_notice(&state, &generous, "").contains("STATE BUDGET"));
    }

    #[test]
    fn the_ledger_slice_only_shows_facts_from_the_range() {
        let mut ledgers = Ledgers::default();
        ledgers
            .commands
            .push(crate::pipeline::ledgers::CommandRecord {
                evt: 5,
                command: "cargo test".into(),
                normalized: "cargo test".into(),
                exit_code: Some(1),
                is_error: Some(true),
                category: crate::pipeline::ledgers::CmdCategory::Test,
                output_head: String::new(),
                output_tail: String::new(),
            });
        assert!(ledger_slice(&ledgers, EvtRange::new(0, 10)).contains("cargo test"));
        assert!(ledger_slice(&ledgers, EvtRange::new(20, 30)).contains("nothing recorded"));
    }
}
