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
use crate::pipeline::ledgers::{CommandRecord, ErrorRecord, ErrorStatus, FileRecord, Ledgers};
use crate::pipeline::mask::Row;
use crate::pipeline::segment::Plan;
use crate::vendor::codex::secrets::RedactMode;
use ops::{EvtRange, OpBatch};
use state::FoldState;

/// What a backend's completion is capped at unless told otherwise. Matches the
/// `max_output_tokens` the fold actually requests.
pub const DEFAULT_MAX_COMPLETION: usize = 4_000;

/// The prompt's size, broken into the terms of ARC (arXiv:2607.25066)
/// Theorem 15: `K = B + M + R + P + Q + η ≤ L = L_ctx − G_max`.
///
/// Named rather than summed because the sum is the only part a reader cannot act
/// on. "The prompt is 61,000 tokens and the window is 32,000" is a fact; knowing
/// that 44,000 of it is the transcript chunk and 9,000 is carried state is what
/// tells you which knob to turn.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize)]
pub struct PromptBudget {
    /// `B` — the fold system prompt.
    pub system: usize,
    /// `M` — carried state, which grows with what has been extracted.
    pub state: usize,
    /// `R` — the deterministic ledger slice, the later index, prior summaries.
    pub ledgers: usize,
    /// `P` — candidates from the isolated premap pass.
    pub premap: usize,
    /// `Q` — the transcript chunk itself.
    pub chunk: usize,
    /// `η` — the template, the schema, and everything else.
    pub scaffold: usize,
}

impl PromptBudget {
    /// `K`.
    pub fn total(&self) -> usize {
        self.system + self.state + self.ledgers + self.premap + self.chunk + self.scaffold
    }
}

/// Fold configuration.
#[derive(Debug, Clone)]
pub struct FoldOptions {
    /// A host's request to stop. Checked before every model call.
    pub cancel: crate::pipeline::Cancel,
    /// Optional user intent that biases extraction.
    pub focus: Option<String>,
    /// Run the isolated premap pass when there are more chunks than this.
    pub premap_threshold: usize,
    /// Parallel premap calls.
    pub concurrency: usize,
    /// Warn the model when the rendered state exceeds this many tokens.
    pub state_tokens: usize,
    pub redact: RedactMode,
    /// The reader's context window, when the caller knows it. With it, every
    /// prompt is checked against `L = model_context − max_completion` *before* it
    /// is sent, so a session too large for the window fails once, immediately,
    /// instead of forty times over two hours.
    pub model_context: Option<usize>,
    /// Tokens reserved for the model's answer (`G_max`).
    pub max_completion: usize,
}

impl Default for FoldOptions {
    fn default() -> Self {
        Self {
            cancel: crate::pipeline::Cancel::new(),
            focus: None,
            premap_threshold: 4,
            concurrency: 4,
            state_tokens: 6_000,
            redact: RedactMode::Default,
            model_context: None,
            max_completion: DEFAULT_MAX_COMPLETION,
        }
    }
}

/// What the fold produced, alongside the state itself.
#[derive(Debug, Clone, Default)]
pub struct FoldReport {
    /// The largest prompt this run sent, in the theorem's terms.
    pub prompt_budget: Option<PromptBudget>,
    pub calls: usize,
    pub repair_calls: usize,
    pub premap_calls: usize,
    pub accepted_ops: usize,
    pub rejected_ops: usize,
    pub input_tokens: u64,
    pub output_tokens: u64,
    /// Model calls that failed outright. Counted separately from the warnings
    /// because "some chunks failed" and "every call failed" are different
    /// situations, and only the caller can decide what the second one means.
    pub failed_calls: usize,
    /// Backend failures that did not abort the run.
    pub warnings: Vec<String>,
}

impl FoldReport {
    /// Keep the largest prompt seen, so the report states the worst case rather
    /// than the first or the last.
    pub fn note_prompt_budget(&mut self, budget: PromptBudget) {
        if self
            .prompt_budget
            .is_none_or(|current| budget.total() > current.total())
        {
            self.prompt_budget = Some(budget);
        }
    }
}

/// Everything the fold needs from the deterministic stages.
#[derive(Debug)]
pub struct FoldInput<'a> {
    pub session: &'a Session,
    pub ledgers: &'a Ledgers,
    pub rows: &'a [Row],
    pub plan: &'a Plan,
    /// The deterministic typed layer. Carried in so that a constraint can be
    /// put in front of the chunk it governs.
    pub triage: &'a crate::pipeline::triage::Triage,
}

/// Run premap, the sequential fold, and the final pass.
pub fn run(
    input: &FoldInput<'_>,
    backend: &dyn Backend,
    options: &FoldOptions,
    progress: crate::pipeline::Progress<'_>,
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
        progress(
            "fold",
            &format!(
                "premapping {} chunk(s), {} at a time",
                input.plan.chunks.len(),
                options.concurrency
            ),
        );
        let notes = premap(input, backend, options, &focus, &mut report)?;
        progress("fold", "premap done; starting the sequential fold");
        notes
    } else {
        vec![String::new(); input.plan.chunks.len()]
    };

    // TypeDecompose (Knowledge Triage §3.3.2): the constraints that govern a
    // chunk are replicated into that chunk rather than left to survive 40
    // sequential model calls by being remembered.
    let scope_vocabulary = crate::pipeline::triage::vocabulary(input.ledgers);
    let gate = |start, end| {
        crate::pipeline::triage::constraints_for(
            input.triage,
            &crate::pipeline::triage::chunk_subsystems(
                input.ledgers,
                start,
                end,
                &scope_vocabulary,
            ),
        )
    };

    // S3b the sequential anchored fold.
    //
    // Reported per chunk. A 40-chunk fold against a real backend runs for hours,
    // and it used to say nothing at all between "40 chunk(s) to fold" and the
    // end — an hour and a half of a silent terminal is indistinguishable from a
    // hang, which is how it was reported.
    let total = input.plan.chunks.len();
    for (index, chunk) in input.plan.chunks.iter().enumerate() {
        // Between chunks, which is where a minute of waiting accumulates.
        options.cancel.check()?;
        progress(
            "fold",
            &format!(
                "chunk {}/{total} (evt {}–{}), {} item(s) so far",
                index + 1,
                chunk.evt_start,
                chunk.evt_end,
                state.active().len()
            ),
        );
        let range = EvtRange::new(chunk.evt_start, chunk.evt_end);
        let rows = &input.rows[chunk.rows.clone()];
        let rows_text = crate::pipeline::mask::render(rows);
        let notes = premap_notes
            .get(index)
            .filter(|notes| !notes.is_empty())
            .map(String::as_str)
            .unwrap_or("");
        let mut chunk_text = rows_text.clone();
        if !notes.is_empty() {
            chunk_text.push_str("\n# CANDIDATES FROM AN ISOLATED PASS OVER THIS CHUNK\n");
            chunk_text.push_str(notes);
        }

        let fields = prompt::Fields {
            chunk_id: chunk.id.clone(),
            session: session_label.clone(),
            focus: budget_notice(&state, options, &focus),
            state: state.render_for_prompt(),
            ledger_slice: format!(
                "{}{}",
                crate::pipeline::triage::governing_block(&gate(chunk.evt_start, chunk.evt_end)),
                ledger_slice(input.ledgers, range)
            ),
            later_index: later_index(input, index),
            prior_summaries: prior_summaries(input.session, range),
            chunk: chunk_text,
            range,
            rejections: String::new(),
            chunk_tokens: approx_tokens(&rows_text),
            premap: approx_tokens(notes),
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
            options,
            true,
        )?;
        state.mark_processed(&chunk.id);
    }

    // S3c the final pass over the recency tail, which the fold never saw.
    if !input.plan.tail.is_empty() {
        options.cancel.check()?;
        progress("fold", "final pass over the recency tail");
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
            ledger_slice: format!(
                "{}{}",
                crate::pipeline::triage::governing_block(&gate(range.start, range.end)),
                last_known_state(input.ledgers)
            ),
            later_index: "(nothing — this is the end of the session)".to_string(),
            prior_summaries: prior_summaries(input.session, range),
            chunk: crate::pipeline::mask::render(rows),
            range,
            rejections: String::new(),
            chunk_tokens: rows.iter().map(|row| row.tokens).sum(),
            premap: 0,
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
            options,
            true,
        )?;
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
    options: &FoldOptions,
    allow_repair: bool,
) -> Result<()> {
    let system = strip(prompt::FOLD_SYSTEM);
    let request = Request {
        role,
        system: system.clone(),
        user: user.to_string(),
        json_schema: serde_json::from_str(prompt::OPS_SCHEMA).ok(),
        max_output_tokens: options.max_completion as u32,
        temperature: Some(0.0),
    };
    let budget = measure_prompt(&system, user, fields);
    enforce_prompt_budget(&budget, chunk_id, options)?;
    report.note_prompt_budget(budget);
    report.calls += 1;
    let response = match backend.complete(&request) {
        Ok(response) => response,
        Err(error) => {
            // A backend failure loses this chunk's semantic pass, not the run:
            // the deterministic parts of the artifact are still correct. It is
            // counted, though, because a run in which *every* call failed has
            // no semantic layer at all and must not look like an ordinary one.
            report.failed_calls += 1;
            report.warnings.push(format!("{chunk_id}: {error}"));
            return Ok(());
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
            return Ok(());
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
        return Ok(());
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
        options,
        false,
    )
}

fn approx_tokens(text: &str) -> usize {
    crate::vendor::codex::truncate::approx_token_count(text)
}

/// Size the prompt in the theorem's terms.
///
/// `fields.chunk_tokens` and `fields.premap` must be the sizes of the text
/// actually placed in `fields.chunk`, split at the seam between the rows and the
/// appended premap notes. Then the six terms sum to the prompt that is sent, and
/// the check is meaningful; if they were independent inputs the sum would be a
/// fiction and the check would refuse working runs.
fn measure_prompt(system: &str, user: &str, fields: &prompt::Fields) -> PromptBudget {
    let tokens = crate::vendor::codex::truncate::approx_token_count;
    let ledgers = tokens(&fields.ledger_slice)
        + tokens(&fields.later_index)
        + tokens(&fields.prior_summaries);
    let named = tokens(&fields.state) + ledgers + fields.premap + fields.chunk_tokens;
    PromptBudget {
        system: tokens(system),
        state: tokens(&fields.state),
        ledgers,
        premap: fields.premap,
        chunk: fields.chunk_tokens,
        // The template, the schema, the focus line, and the separators, by
        // subtraction — so the six terms always sum to the prompt actually sent.
        // The check is only meaningful if they do.
        scaffold: tokens(user).saturating_sub(named),
    }
}

/// `K ≤ L`, checked before the call rather than discovered by it.
///
/// Only enforced when the caller said what the window is. Without it the check
/// would be a guess, and a guess that fails a working run is worse than no check.
fn enforce_prompt_budget(
    budget: &PromptBudget,
    chunk_id: &str,
    options: &FoldOptions,
) -> Result<()> {
    let Some(context) = options.model_context else {
        return Ok(());
    };
    let limit = context.saturating_sub(options.max_completion);
    let k = budget.total();
    if k <= limit {
        return Ok(());
    }
    Err(crate::error::Error::Usage(format!(
        "the prompt for {chunk_id} is {k} tokens but the reader's window leaves {limit} \
         (--model-context {context} minus --max-completion {}).\n\
         The prompt is {} system + {} state + {} ledgers + {} premap + {} chunk + {} scaffold.\n\
         Fix one of: raise --model-context, lower --chunk-tokens (the chunk term is the one that \
         scales with the session), or use --llm none for the deterministic artifact, which needs \
         no window at all.",
        options.max_completion,
        budget.system,
        budget.state,
        budget.ledgers,
        budget.premap,
        budget.chunk,
        budget.scaffold,
    )))
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
) -> Result<Vec<String>> {
    let chunks = &input.plan.chunks;
    let concurrency = options.concurrency.clamp(1, 16);
    let mut notes = vec![String::new(); chunks.len()];
    let mut warnings: Vec<String> = Vec::new();
    let mut calls = 0usize;

    for (batch_index, batch) in chunks.chunks(concurrency).enumerate() {
        options.cancel.check()?;
        let offset = batch_index * concurrency;
        // Built and checked before anything is spawned. Premap used to construct
        // its own `Request` and call the backend directly, so the window check
        // saw 41 of this run's 81 calls and let the other 40 go out unmeasured.
        let system = strip(prompt::FOLD_SYSTEM);
        let prepared: Vec<(usize, String)> = batch
            .iter()
            .enumerate()
            .map(|(index, chunk)| {
                let range = EvtRange::new(chunk.evt_start, chunk.evt_end);
                let rows = &input.rows[chunk.rows.clone()];
                let text = crate::pipeline::mask::render(rows);
                let fields = prompt::Fields {
                    chunk_id: chunk.id.clone(),
                    focus: focus.to_string(),
                    ledger_slice: ledger_slice(input.ledgers, range),
                    chunk_tokens: approx_tokens(&text),
                    chunk: text,
                    range,
                    ..Default::default()
                };
                let user = prompt::render(prompt::Template::Premap, &fields, prompt::OPS_SCHEMA);
                let budget = measure_prompt(&system, &user, &fields);
                enforce_prompt_budget(&budget, &chunk.id, options)?;
                report.note_prompt_budget(budget);
                Ok((offset + index, user))
            })
            .collect::<Result<Vec<_>>>()?;

        let results: Vec<(usize, std::result::Result<String, String>)> =
            std::thread::scope(|scope| {
                let handles: Vec<_> = prepared
                    .into_iter()
                    .map(|(index, user)| {
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
                                index,
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
                Err(error) => {
                    report.failed_calls += 1;
                    warnings.push(format!("premap chunk {index}: {error}"));
                }
            }
        }
    }

    report.premap_calls = calls;
    report.calls += calls;
    report.warnings.extend(warnings);
    Ok(notes)
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
/// Tokens the deterministic ledger slice may spend in one prompt.
///
/// The per-chunk slice is exhaustive on purpose: a 24k-token chunk wants every
/// file and command inside its own range. A chunk that spans the *whole* session
/// turns that into the whole ledger, and on a 103,757-event session that is
/// 363,066 tokens of files, commands and errors sitting inside a prompt whose
/// transcript is only 52,964. The cap is what keeps `--mode fast` from being
/// slower than the mode it replaced.
pub const LEDGER_BUDGET: usize = 12_000;

/// One ledger line's own limit, so a single pasted stack trace cannot spend the
/// whole slice.
const LEDGER_ITEM_TOKENS: usize = 200;

fn ledger_slice(ledgers: &Ledgers, range: EvtRange) -> String {
    // Ranked before it is capped, because a cap on an unranked list keeps
    // whichever entries happened to be first in the ledger.
    let mut files: Vec<(&FileRecord, u32)> = ledgers
        .files
        .iter()
        .filter(|file| file.last_evt >= range.start && file.first_evt <= range.end)
        .map(|file| (file, file.edits + file.reads))
        .collect();
    files.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.path.cmp(&b.0.path)));
    let files: Vec<String> = files
        .into_iter()
        .map(|(file, _)| {
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

    let mut commands: Vec<&CommandRecord> = ledgers
        .commands
        .iter()
        .filter(|command| range.contains(command.evt))
        .collect();
    commands.sort_by_key(|command| std::cmp::Reverse(command.evt));
    let commands: Vec<String> = commands
        .into_iter()
        .map(|command| {
            format!(
                "`{}` → {} [evt {}]",
                command.normalized,
                command.status(),
                command.evt
            )
        })
        .collect();

    let mut errors: Vec<&ErrorRecord> = ledgers
        .errors
        .iter()
        .filter(|error| error.last_evt >= range.start && error.first_evt <= range.end)
        .collect();
    errors.sort_by(|a, b| {
        (a.status != ErrorStatus::Unresolved)
            .cmp(&(b.status != ErrorStatus::Unresolved))
            .then_with(|| b.occurrences.cmp(&a.occurrences))
            .then_with(|| a.sig.cmp(&b.sig))
    });
    let errors: Vec<String> = errors
        .into_iter()
        .map(|error| {
            format!(
                "{} ×{} — {}",
                error.sig,
                error.occurrences,
                one_line(error.example.lines().next().unwrap_or(""))
            )
        })
        .collect();

    let dead_ends: Vec<String> = ledgers
        .dead_end_candidates()
        .iter()
        .filter(|error| error.last_evt >= range.start && error.first_evt <= range.end)
        .map(|error| format!("{} (×{}, still unresolved)", error.sig, error.occurrences))
        .collect();

    let mut out = String::new();
    let mut remaining = LEDGER_BUDGET;
    emit_ledger(&mut out, &mut remaining, "Files touched", &files);
    emit_ledger(&mut out, &mut remaining, "Commands", &commands);
    emit_ledger(&mut out, &mut remaining, "Error signatures", &errors);
    emit_ledger(
        &mut out,
        &mut remaining,
        "Repeatedly unresolved (dead-end candidates)",
        &dead_ends,
    );

    if out.is_empty() {
        "(nothing recorded for this range)".to_string()
    } else {
        out
    }
}

/// Emit one ledger section under a shared budget, saying what it left out.
///
/// The first entry is always kept, so a section is never rendered as empty while
/// entries exist — an empty "Error signatures" line reads as "there were none",
/// which is the one thing it must never mean.
fn emit_ledger(out: &mut String, remaining: &mut usize, label: &str, items: &[String]) {
    if items.is_empty() {
        return;
    }
    let mut kept: Vec<String> = Vec::new();
    for item in items {
        let item = crate::vendor::codex::truncate::truncate_middle_tokens(item, LEDGER_ITEM_TOKENS);
        let cost = approx_tokens(&item) + 2;
        if cost > *remaining && !kept.is_empty() {
            break;
        }
        *remaining = remaining.saturating_sub(cost);
        kept.push(item);
    }
    let dropped = items.len() - kept.len();
    out.push_str(&format!("{label}: {}", kept.join("; ")));
    if dropped > 0 {
        out.push_str(&format!(" … and {dropped} more"));
    }
    out.push('\n');
}

/// The first line of a block of output, collapsed.
fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<&str>>().join(" ")
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

/// The provider's own compaction summaries, as low-trust seeds for this chunk.
///
/// A provider compacts when it is out of room, so its summary is all that
/// survives of what it chose to discard — lossy, written for a different
/// purpose, and sometimes wrong. It is worth showing the fold, because a chunk
/// several turns after the boundary would otherwise see nothing of what came
/// before it; it is not worth trusting, which is what the prompt says.
///
/// Only summaries strictly before this chunk are eligible, newest last, capped
/// so a heavily compacted session cannot push the transcript out of the prompt.
/// A boundary whose text the provider kept encrypted, or never wrote, still
/// appears — the fold should know that history exists and is unreadable.
fn prior_summaries(session: &Session, range: EvtRange) -> String {
    /// More than this and the block starts competing with the chunk itself.
    const MAX: usize = 3;
    /// Per-summary cap; roughly the size of a chunk-level synopsis.
    const TOKENS: usize = 400;

    let mut eligible: Vec<&crate::ir::NativeCompaction> = session
        .native_compactions
        .iter()
        .filter(|compaction| compaction.evt < range.start)
        .collect();
    // Newest last, so the model reads them in the order they happened.
    eligible.sort_by_key(|compaction| compaction.evt);
    let start = eligible.len().saturating_sub(MAX);

    if eligible.is_empty() {
        return "None before this point.".to_string();
    }

    eligible[start..]
        .iter()
        .map(|compaction| {
            let body = match &compaction.summary {
                Some(text) => crate::vendor::codex::truncate::truncate_middle_tokens(text, TOKENS),
                None => {
                    "(the provider recorded this boundary but left no readable summary)".to_string()
                }
            };
            format!("- [evt {}] {}", compaction.evt, body.replace('\n', " "))
        })
        .collect::<Vec<String>>()
        .join("\n")
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

    /// Parse the fixture that carries both kinds of Codex compaction.
    fn windowed_compaction_session() -> Session {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/codex/windowed-compaction.jsonl");
        crate::adapters::parse_path(&path, 0.5).expect("the fixture parses")
    }

    #[test]
    fn prior_summaries_are_offered_as_seeds_only_after_they_happened() {
        let session = windowed_compaction_session();

        // The first chunk starts before any compaction: nothing to seed.
        assert_eq!(
            prior_summaries(&session, EvtRange::new(0, 0)),
            "None before this point."
        );

        // A chunk after the legacy reset at evt 1 sees its summary...
        let seeded = prior_summaries(&session, EvtRange::new(3, 5));
        assert!(seeded.contains("cargo-dist"), "{seeded}");
        assert!(seeded.contains("[evt 1]"), "{seeded}");
        // ...and not the window marker at evt 3, which has not happened yet.
        assert_eq!(seeded.lines().count(), 1, "{seeded}");

        // A chunk after the window marker sees both. The marker kept no text,
        // so it is reported as an unreadable boundary rather than dropped: the
        // fold should still know that history it cannot see exists.
        let later = prior_summaries(&session, EvtRange::new(4, 5));
        assert!(later.contains("[evt 1]"), "{later}");
        assert!(later.contains("[evt 3]"), "{later}");
        assert!(later.contains("no readable summary"), "{later}");
    }

    #[test]
    fn the_low_trust_seed_reaches_the_rendered_fold_prompt() {
        let session = windowed_compaction_session();
        let range = EvtRange::new(4, 5);
        let fields = prompt::Fields {
            prior_summaries: prior_summaries(&session, range),
            range,
            ..Default::default()
        };
        let rendered = prompt::render(prompt::Template::FoldUser, &fields, "{}");

        // The framing matters as much as the text: the fold must be told these
        // are a hint, not evidence, or it will restate a provider summary as a
        // decision with no source of its own.
        assert!(rendered.contains("PRIOR PROVIDER SUMMARIES"), "{rendered}");
        assert!(rendered.contains("never as evidence"), "{rendered}");
        assert!(rendered.contains("cargo-dist"), "{rendered}");
        assert!(!rendered.contains("{{prior_summaries}}"), "{rendered}");
    }

    #[test]
    fn prior_summaries_are_capped_and_kept_in_chronological_order() {
        let mut session = windowed_compaction_session();
        session.native_compactions = (0..6)
            .map(|n| crate::ir::NativeCompaction {
                evt: n * 10,
                summary: Some(format!("summary {n}")),
                windowed: false,
            })
            .collect();

        let rendered = prior_summaries(&session, EvtRange::new(100, 110));
        let lines: Vec<&str> = rendered.lines().collect();
        // Three most recent, oldest first.
        assert_eq!(lines.len(), 3, "{rendered}");
        assert!(lines[0].contains("summary 3"), "{rendered}");
        assert!(lines[2].contains("summary 5"), "{rendered}");
        assert!(!rendered.contains("summary 2"), "{rendered}");
    }

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
            &FoldOptions::default(),
            true,
        )
        .expect("no window was given, so nothing to refuse");
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
            &FoldOptions::default(),
            true,
        )
        .expect("no window was given, so nothing to refuse");
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
            &FoldOptions::default(),
            true,
        )
        .expect("no window was given, so nothing to refuse");
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
    fn the_five_named_terms_sum_to_the_prompt_that_is_sent() {
        // The check is only worth anything if the terms describe the real prompt.
        // Self-consistent the way production is: `chunk_tokens` and `premap` are
        // the sizes of the text actually placed in `chunk`, split at the seam
        // between the rows and the premap notes.
        let notes = "candidate one; candidate two";
        let chunk = format!("the chunk\n# CANDIDATES\n{notes}");
        let fields = prompt::Fields {
            state: "carried state".into(),
            ledger_slice: "files: a.rs".into(),
            later_index: "later".into(),
            prior_summaries: "summary".into(),
            chunk,
            chunk_tokens: approx_tokens("the chunk"),
            premap: approx_tokens(notes),
            ..Default::default()
        };
        // The real rendered prompt, because the invariant is about the prompt
        // that is sent: if `user` were a stub the terms could not describe it.
        let user = prompt::render(prompt::Template::FoldUser, &fields, prompt::OPS_SCHEMA);
        let system = "system prompt";
        let budget = measure_prompt(system, &user, &fields);
        assert_eq!(
            budget.total(),
            approx_tokens(system) + approx_tokens(&user),
            "the terms must sum to the prompt: {budget:?}"
        );
        assert!(budget.scaffold > 0, "the template has to cost something");
        assert_eq!(budget.chunk, fields.chunk_tokens);
        assert_eq!(budget.premap, fields.premap);
    }

    #[test]
    fn a_prompt_that_cannot_fit_the_window_fails_before_it_is_sent() {
        let fields = prompt::Fields {
            chunk_tokens: 50_000,
            ..Default::default()
        };
        let budget = measure_prompt("s", "u", &fields);
        // A 16k window with 4k reserved leaves 12k, and the chunk alone is 50k.
        let options = FoldOptions {
            model_context: Some(16_000),
            ..Default::default()
        };
        let error = enforce_prompt_budget(&budget, "c7", &options).expect_err("must refuse");
        let message = error.to_string();
        assert!(message.contains("c7"), "{message}");
        assert!(message.contains("--chunk-tokens"), "{message}");

        // No window given means no check, not a guessed one.
        let unchecked = FoldOptions::default();
        assert!(enforce_prompt_budget(&budget, "c7", &unchecked).is_ok());
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

#[cfg(test)]
mod failing_backend_tests {
    use super::*;
    use crate::llm::{Capabilities, Request, Response};
    use crate::pipeline::mask::Row;
    use crate::pipeline::segment::{Chunk, Plan};

    /// A backend that never answers, which is what a rate-limited subscription
    /// looks like from here.
    struct AlwaysFails;

    impl Backend for AlwaysFails {
        fn name(&self) -> String {
            "cli:claude".to_string()
        }
        fn capabilities(&self) -> Capabilities {
            Capabilities {
                json_schema_native: false,
                max_context: None,
            }
        }
        fn complete(&self, _request: &Request) -> crate::error::Result<Response> {
            Err(crate::error::Error::LlmFailed {
                backend: "cli:claude".to_string(),
                message: "exited with 1: You've hit your session limit".to_string(),
            })
        }
    }

    #[test]
    fn a_failed_backend_call_is_counted_rather_than_only_warned_about() {
        // The run that produced an empty artifact: every call failed, every
        // failure became a warning, and nothing counted them.
        let session = crate::ir::Session {
            agent: crate::ir::AgentKind::ClaudeCode,
            id: "id".into(),
            source_paths: Vec::new(),
            source_hash: String::new(),
            meta: crate::ir::SessionMeta::default(),
            events: Vec::new(),
            active: Vec::new(),
            native_compactions: Vec::new(),
            diagnostics: Vec::new(),
        };
        let ledgers = crate::pipeline::ledgers::Ledgers::default();
        let rows = vec![Row {
            evt: 1,
            tier: crate::vendor::codex::tiered_input::Tier::User,
            text: "do the thing".into(),
            tokens: 3,
            is_human_turn: true,
        }];
        let plan = Plan {
            episodes: Vec::new(),
            chunks: vec![Chunk {
                id: "c0".into(),
                rows: 0..1,
                evt_start: 1,
                evt_end: 1,
                tokens: 3,
                episode_ids: Vec::new(),
            }],
            tail: 0..0,
            tail_episode_ids: Vec::new(),
        };
        let triage = crate::pipeline::triage::Triage::default();
        let input = FoldInput {
            session: &session,
            ledgers: &ledgers,
            rows: &rows,
            plan: &plan,
            triage: &triage,
        };
        let options = FoldOptions {
            // One chunk, so there is no premap to confuse the count.
            premap_threshold: usize::MAX,
            ..Default::default()
        };
        let (state, report) = run(&input, &AlwaysFails, &options, &mut |_, _| {})
            .expect("a failed call is not fatal");

        assert_eq!(report.calls, 1);
        assert_eq!(report.failed_calls, 1, "the failure must be counted");
        assert!(report.accepted_ops == 0);
        assert!(state.items.is_empty());
        assert!(
            report.warnings.iter().any(|w| w.contains("session limit")),
            "and the reason must survive: {:?}",
            report.warnings
        );
    }
}
