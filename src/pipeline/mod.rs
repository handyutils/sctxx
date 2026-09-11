//! The extraction pipeline, S0 through S7 (spec §4.4).
//!
//! ```text
//! session file(s) ─► S0 adapter ─► IR Session (events, active branch)
//!                        │
//!         ┌────────────────┼───────────────────┐
//!         ▼                    ▼                     ▼
//!   S1 ledgers          S2 mask + segment        S4 tail
//!         │                    │                     │
//!         └─────────────► S3 anchored fold ◄────────┘
//!                              │
//!                       S5 reconcile ─► S7 render
//! ```
//!
//! S6 (the probe loop) is roadmap M5 and not built yet; `--mode full`
//! therefore behaves as `standard` and says so on stderr.

pub mod artifact;
pub mod finalize;
pub mod fold;
pub mod ledgers;
pub mod mask;
pub mod reconcile;
pub mod render;
pub mod segment;
pub mod triage;

use crate::adapters::{self, discovery::SessionSummary};
use crate::error::{Error, Result};
use crate::ir::{AgentKind, EventIdx, Session};
use crate::llm::Selection;
use crate::vendor::codex::secrets::RedactMode;
use serde::Serialize;
use std::path::{Path, PathBuf};

/// How much of the pipeline to run (spec §3.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Mode {
    /// One pass, tier-budgeted; no premap.
    Fast,
    /// Premap when it pays for itself, then the sequential fold.
    Standard,
    /// Standard plus the probe loop (roadmap M5; currently equals standard).
    Full,
}

impl Mode {
    pub fn label(self) -> &'static str {
        match self {
            Mode::Fast => "fast",
            Mode::Standard => "standard",
            Mode::Full => "full",
        }
    }

    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "fast" => Ok(Mode::Fast),
            "standard" => Ok(Mode::Standard),
            "full" => Ok(Mode::Full),
            other => Err(Error::Usage(format!(
                "unknown mode `{other}` (expected fast, standard, or full)"
            ))),
        }
    }
}

/// Everything `sctxx extract` needs.
#[derive(Debug, Clone)]
pub struct ExtractOptions {
    pub mode: Mode,
    pub llm: Selection,
    pub budget: usize,
    pub tail_tokens: usize,
    pub chunk_tokens: usize,
    pub focus: Option<String>,
    pub repo: Option<PathBuf>,
    pub verify: bool,
    pub strict: bool,
    pub layers: render::Layers,
    pub include_sidechains: bool,
    pub since_compact: bool,
    pub keep_reasoning: bool,
    pub keep_system: bool,
    pub redact: RedactMode,
    pub concurrency: usize,
    pub max_bad_line_rate: f64,
}

impl Default for ExtractOptions {
    fn default() -> Self {
        Self {
            mode: Mode::Standard,
            llm: Selection::Auto,
            budget: 8_000,
            tail_tokens: 12_000,
            chunk_tokens: 24_000,
            focus: None,
            repo: None,
            verify: true,
            strict: false,
            layers: render::Layers::default(),
            include_sidechains: false,
            since_compact: false,
            keep_reasoning: false,
            keep_system: false,
            redact: RedactMode::Default,
            concurrency: 4,
            max_bad_line_rate: adapters::DEFAULT_MAX_BAD_LINE_RATE,
        }
    }
}

/// Diagnostics and accounting for one run (`report.json`).
#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub sctxx: &'static str,
    pub mode: &'static str,
    pub llm: String,
    pub session: String,
    pub agent: &'static str,
    pub events: usize,
    pub active_events: usize,
    pub user_turns: usize,
    pub kind_counts: std::collections::BTreeMap<&'static str, usize>,
    pub rows: usize,
    pub episodes: usize,
    pub chunks: usize,
    pub tail_rows: usize,
    pub tokens: render::TokenCounts,
    pub fold_calls: usize,
    pub premap_calls: usize,
    pub repair_calls: usize,
    pub accepted_ops: usize,
    pub rejected_ops: usize,
    /// Whether the model-written layer is in the artifact, and why not.
    pub semantic_state: SemanticState,
    /// The deterministic typed layer, and what became of it.
    pub triage: triage::Triage,
    pub guard: triage::GuardReport,
    pub fold_failed_calls: usize,
    pub llm_input_tokens: u64,
    pub llm_output_tokens: u64,
    pub warnings: Vec<String>,
    pub diagnostics: Vec<crate::ir::Diagnostic>,
    pub verification: reconcile::Reconciliation,
    pub elapsed_ms: u128,
}

/// What an empty semantic layer means, given how the fold went.
///
/// A separate function because this is the judgement that was wrong: 81 failed
/// calls, zero items, and an artifact that rendered as an ordinary standard
/// handoff.
pub fn classify_semantic(report: &fold::FoldReport) -> SemanticState {
    if report.calls == 0 {
        return SemanticState::NotRequested;
    }
    if report.failed_calls == report.calls {
        // Every call failed, so there was never a semantic pass to lose.
        return SemanticState::Unavailable;
    }
    if report.accepted_ops == 0 {
        // The calls worked and produced nothing usable. A different problem, and
        // a different thing to tell the reader.
        return SemanticState::Degraded;
    }
    SemanticState::Ok
}

/// Whether the model-written semantic layer is actually in the artifact.
///
/// The deterministic artifact is complete on its own — every `[evt a–b]`
/// pointer, every ledger, the recency tail — but a *standard-mode* artifact
/// renders sections (hard constraints, current step, decisions, open threads)
/// that only the fold can fill. An empty semantic layer therefore has to be
/// distinguishable from a session that genuinely had nothing to say, and from a
/// run where a model was never asked. Rendering all three the same way is how a
/// run in which every model call failed came to look like a normal handoff.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SemanticState {
    /// No model was asked for. The deterministic artifact is the product.
    NotRequested,
    /// The fold produced items.
    Ok,
    /// The fold ran and produced nothing usable.
    Degraded,
    /// Every model call failed, so there is no semantic layer at all.
    Unavailable,
}

/// Serialised as its label, so `report.json` says `"semantic_state":
/// "unavailable"` rather than an enum shape a reader has to know.
impl serde::Serialize for SemanticState {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(self.label())
    }
}

impl SemanticState {
    pub fn label(self) -> &'static str {
        match self {
            SemanticState::NotRequested => "not_requested",
            SemanticState::Ok => "ok",
            SemanticState::Degraded => "degraded",
            SemanticState::Unavailable => "unavailable",
        }
    }

    /// What a reader of the artifact must be told, when it is not `Ok`.
    pub fn notice(self) -> Option<&'static str> {
        match self {
            // Said in L0, not only in the header. A reader who does not know the
            // semantic layer is absent will read a transcript digest as though it
            // were a handoff — which is exactly what happened. Kept to two lines,
            // because the rest of L0 is what they came for.
            SemanticState::NotRequested => Some(
                "No model ran. There is no model-written state: no goals, decisions, dead ends or \
                 current step, and only the standing instructions a pattern could find — a rule \
                 stated declaratively is absent rather than absent-minded. What follows is evidence \
                 rather than understanding. For the semantic layer: `sctxx extract <ref> --llm \
                 cli:<agent>`.",
            ),
            SemanticState::Ok => None,
            SemanticState::Degraded => Some(
                "The model-written state is EMPTY. The fold ran and produced no items, so the \
                 sections it would fill are absent rather than empty. Read the ledgers and the \
                 recency tail, and verify against the repository before acting.",
            ),
            SemanticState::Unavailable => Some(
                "The model-written state is UNAVAILABLE: every model call failed. What follows \
                 is the deterministic artifact — ledgers, pointers and the recency tail — which \
                 is complete for following the evidence, but nothing here was summarised by a \
                 model. Read the tail before acting.",
            ),
        }
    }
}

/// The result of an extraction.
#[derive(Debug)]
pub struct Extraction {
    pub session: Session,
    pub ledgers: ledgers::Ledgers,
    pub rows: Vec<mask::Row>,
    pub plan: segment::Plan,
    pub state: fold::state::FoldState,
    pub reconciliation: reconcile::Reconciliation,
    /// The deterministic typed layer: standing instructions found by pattern.
    pub triage: triage::Triage,
    /// What became of each of them, checked after the semantic pass.
    pub guard: triage::GuardReport,
    /// The reconciled end state: what the evidence says is true *now*.
    pub end_state: finalize::EndState,
    pub report: Report,
    pub llm_label: String,
}

impl Extraction {
    /// The rows held back as the recency tail.
    pub fn tail(&self) -> &[mask::Row] {
        &self.rows[self.plan.tail.clone()]
    }

    /// Render the markdown artifact.
    pub fn markdown(&self, options: &ExtractOptions) -> String {
        let mut render_options = self.render_options(options);
        // The header states the artifact's own size, so it has to be measured
        // before it can be printed. One extra render is cheap next to the
        // parse, and the first pass differs from the second only by the few
        // bytes its own digit count changes — far below the 4-bytes-per-token
        // precision of the estimate itself.
        let measured = render::markdown(&self.artifact(&render_options));
        render_options.artifact_tokens =
            crate::vendor::codex::truncate::approx_token_count(&measured);
        render::markdown(&self.artifact(&render_options))
    }

    /// Render the JSON artifact.
    pub fn json(&self, options: &ExtractOptions) -> serde_json::Value {
        let render_options = self.render_options(options);
        serde_json::to_value(render::json(&self.artifact(&render_options)))
            .unwrap_or(serde_json::Value::Null)
    }

    fn artifact<'a>(&'a self, options: &'a render::RenderOptions) -> render::Artifact<'a> {
        render::Artifact {
            session: &self.session,
            ledgers: &self.ledgers,
            state: &self.state,
            reconciliation: &self.reconciliation,
            tail: self.tail(),
            options,
        }
    }

    fn render_options(&self, options: &ExtractOptions) -> render::RenderOptions {
        render::RenderOptions {
            budget: options.budget,
            layers: options.layers,
            mode: options.mode.label(),
            llm: self.llm_label.clone(),
            end_state: Some(self.end_state.clone()),
            // The artifact says whether the model-written layer is there, so a
            // reader never has to infer it from absent sections.
            semantic: self.report.semantic_state,
            triage: Some(self.triage.clone()),
            guard: self.guard.clone(),
            redact: options.redact,
            // The header reports what the whole session costs as a masked
            // transcript; only the pipeline knows that number.
            masked_tokens: self.report.tokens.masked,
            ..render::RenderOptions::default()
        }
    }
}

/// Progress notifications. Extraction can take minutes with an LLM backend, so
/// the caller decides how to surface them (text or NDJSON on stderr).
pub type Progress<'a> = &'a mut dyn FnMut(&str, &str);

/// A flag a host can set to stop a running extraction.
///
/// Checked at every stage boundary and before every model call, which is where
/// the time goes: an extraction that spends minutes in the fold must be
/// stoppable between its chunks, not only between its stages.
#[derive(Debug, Clone, Default)]
pub struct Cancel(std::sync::Arc<std::sync::atomic::AtomicBool>);

impl Cancel {
    pub fn new() -> Self {
        Self::default()
    }

    /// Ask the running extraction to stop at its next boundary.
    pub fn cancel(&self) {
        self.0.store(true, std::sync::atomic::Ordering::SeqCst);
    }

    pub fn cancelled(&self) -> bool {
        self.0.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Stop here if someone asked.
    pub fn check(&self) -> Result<()> {
        if self.cancelled() {
            Err(Error::Cancelled)
        } else {
            Ok(())
        }
    }
}

/// Where a `--since-compact` run starts.
///
/// A *legacy* compaction is a real history reset: everything before it is gone
/// from the provider's own view, so the newest one is the honest starting
/// point. A *window re-anchor* kept the transcript, so the earliest one is the
/// point after which the session is still complete. Prefer a reset when the
/// session has one; otherwise use the earliest re-anchor.
///
/// Returns `None` when the provider never compacted, in which case the whole
/// session is the correct answer.
///
/// See `docs/adr/0002-codex-compaction-algorithm-reuse.md`.
fn since_compact_boundary(session: &Session) -> Option<EventIdx> {
    let reset = session
        .native_compactions
        .iter()
        .filter(|compaction| !compaction.windowed)
        .map(|compaction| compaction.evt)
        .max();
    reset.or_else(|| {
        session
            .native_compactions
            .iter()
            .filter(|compaction| compaction.windowed)
            .map(|compaction| compaction.evt)
            .min()
    })
}

/// Run the pipeline for an already-resolved session file.
pub fn extract(
    summary: &SessionSummary,
    options: &ExtractOptions,
    progress: Progress<'_>,
) -> Result<Extraction> {
    extract_interruptible(summary, options, progress, &Cancel::new())
}

/// `extract`, with a way to stop it.
///
/// The CLI calls [`extract`] and therefore cannot be interrupted; the TUI calls
/// this and can.
pub fn extract_interruptible(
    summary: &SessionSummary,
    options: &ExtractOptions,
    progress: Progress<'_>,
    cancel: &Cancel,
) -> Result<Extraction> {
    let started = std::time::Instant::now();
    cancel.check()?;

    // S0 — parse and resolve the active branch.
    progress("parse", &format!("reading {}", summary.path.display()));
    let agent = AgentKind::from_slug(summary.agent)
        .ok_or_else(|| Error::UnknownFormat(summary.path.clone()))?;
    let source = adapters::source::read(&summary.path)?;

    // Newer Claude Code versions store each subagent in its own file beside
    // the session. They are read only on request: their spawn and result are
    // already in the main conversation, and they can be large.
    let sidechains = if options.include_sidechains {
        let paths = adapters::discovery::sidechain_paths(agent, &summary.path);
        if !paths.is_empty() {
            progress(
                "parse",
                &format!("including {} subagent transcript(s)", paths.len()),
            );
        }
        paths
            .iter()
            .filter_map(|path| adapters::source::read(path).ok())
            .collect()
    } else {
        Vec::new()
    };

    let mut session = if sidechains.is_empty() {
        adapters::parse_as(agent, source, options.max_bad_line_rate)?
    } else {
        adapters::parse_with_sidechains(agent, source, sidechains, options.max_bad_line_rate)?
    };
    progress(
        "parse",
        &format!(
            "{} events, {} on the active branch, {} user turns",
            session.events.len(),
            session.active.len(),
            session.user_turns()
        ),
    );

    // `--since-compact` (spec §3.4): keep the transcript from the newest
    // provider compaction boundary onward, retaining that boundary's own event
    // as the low-trust seed. `events` is untouched, so every `[evt a-b]`
    // pointer still resolves and `expand` can still reach the earlier history.
    if options.since_compact {
        match since_compact_boundary(&session) {
            Some(boundary) => {
                let before = session.active.len();
                session.active.retain(|idx| *idx >= boundary);
                session
                    .native_compactions
                    .retain(|compaction| compaction.evt >= boundary);
                progress(
                    "since-compact",
                    &format!(
                        "starting at evt {boundary}: {} of {before} active events kept",
                        session.active.len()
                    ),
                );
            }
            None => progress(
                "since-compact",
                "--since-compact was given but this session has no provider compaction; using the whole session",
            ),
        }
    }

    cancel.check()?;

    // S1 — deterministic ledgers.
    let ledgers = ledgers::build(&session, options.redact);
    progress(
        "ledgers",
        &format!(
            "{} files, {} commands, {} error signatures",
            ledgers.files.len(),
            ledgers.commands.len(),
            ledgers.errors.len()
        ),
    );

    // S1b — deterministic typed extraction.
    //
    // Before the mask, before the segment, and before any model call, because
    // the result is what the fold is *given* rather than what it is asked to
    // find. `ItemKind::Constraint` is first in the rendering priority and the
    // artifact tells its reader to treat it as binding; it must therefore exist
    // whether or not a model runs.
    let triage = triage::run(&session, &ledgers, options.redact);
    progress(
        "triage",
        &format!(
            "{} standing instruction(s) in {} user message(s){}",
            triage.constraints.len(),
            triage.messages,
            if triage.dropped > 0 {
                format!(", {} beyond the cap", triage.dropped)
            } else {
                String::new()
            }
        ),
    );

    // S2/S4 — mask, segment, and split off the recency tail.
    let mask_options = mask::MaskOptions {
        reasoning: if options.keep_reasoning {
            mask::ReasoningPolicy::Keep
        } else {
            mask::ReasoningPolicy::Drop
        },
        keep_system: options.keep_system,
        include_sidechains: options.include_sidechains,
        redact: options.redact,
    };
    let rows = mask::build(&session, &mask_options);
    let masked_tokens: usize = rows.iter().map(|row| row.tokens).sum();
    let segment_options = segment::SegmentOptions {
        chunk_tokens: options.chunk_tokens,
        tail_tokens: options.tail_tokens,
        ..Default::default()
    };
    let plan = segment::plan(&rows, &segment_options);
    progress(
        "segment",
        &format!(
            "{} rows, {} episodes, {} chunks to fold, {} rows in the tail",
            rows.len(),
            plan.episodes.len(),
            plan.chunks.len(),
            plan.tail.len()
        ),
    );

    // S3 — the anchored fold, when a backend is available.
    let mut state = fold::state::FoldState::new();
    triage::seed(&mut state, &triage);
    let mut fold_report = fold::FoldReport::default();
    let backend = crate::llm::build(&options.llm)?;
    let llm_label = match &backend {
        Some(backend) => backend.name(),
        None => "none".to_string(),
    };
    // Whether the semantic layer is really there. Decided after the fold, and
    // carried into the artifact so a reader is never shown an empty state as if
    // it were a full one.
    let mut semantic = SemanticState::NotRequested;

    if let Some(backend) = &backend {
        if plan.chunks.is_empty() && plan.tail.is_empty() {
            progress("fold", "nothing to fold: the session has no usable rows");
        } else {
            progress(
                "fold",
                &format!(
                    "{} chunk(s) plus a final pass via {}",
                    plan.chunks.len(),
                    llm_label
                ),
            );
            let fold_options = fold::FoldOptions {
                cancel: cancel.clone(),
                focus: options.focus.clone(),
                // `fast` never premaps; the point of fast is one pass.
                premap_threshold: if options.mode == Mode::Fast {
                    usize::MAX
                } else {
                    4
                },
                concurrency: options.concurrency,
                redact: options.redact,
                ..Default::default()
            };
            let input = fold::FoldInput {
                session: &session,
                ledgers: &ledgers,
                rows: &rows,
                plan: &plan,
                triage: &triage,
            };
            let (folded, report) = fold::run(&input, backend.as_ref(), &fold_options, progress)?;
            state = folded;
            fold_report = report;

            semantic = classify_semantic(&fold_report);
            if matches!(
                semantic,
                SemanticState::Degraded | SemanticState::Unavailable
            ) {
                let detail = fold_report
                    .warnings
                    .first()
                    .map(String::as_str)
                    .unwrap_or("no reason reported");
                progress(
                    "semantic",
                    &format!(
                        "{} of {} model calls failed; the model-written state is {}. First failure: {detail}",
                        fold_report.failed_calls,
                        fold_report.calls,
                        semantic.label()
                    ),
                );
            }
            progress(
                "fold",
                &format!(
                    "{} operations accepted, {} rejected, {} item(s) active",
                    fold_report.accepted_ops,
                    fold_report.rejected_ops,
                    state.active().len()
                ),
            );
        }
    } else {
        progress(
            "fold",
            "skipped: no LLM backend. The deterministic artifact is still complete.",
        );
    }

    // The deterministic post-compaction verifier (Knowledge Triage §3.3.1). It
    // runs on the *state* the fold produced, which is where a constraint is
    // actually lost: the paper measures a run without this check reporting
    // apparent 1.00 constraint recall while silently dropping a mean 57 % of the
    // constraints that should have been kept.
    let guard = triage::verify(&mut state, &triage);
    if guard.restored > 0 {
        progress(
            "triage",
            &format!(
                "{} of {} constraint(s) were dropped by the semantic pass and restored",
                guard.restored, guard.found
            ),
        );
    }
    if !guard.is_clean() {
        progress(
            "triage",
            &format!(
                "{} constraint(s) could not be restored: {}",
                guard.missing.len(),
                guard.missing.join("; ")
            ),
        );
    }

    // S5 — reconcile against the repository.
    let repo = options
        .repo
        .clone()
        .or_else(|| session.meta.cwd.clone().filter(|cwd| cwd.is_dir()))
        .or_else(|| std::env::current_dir().ok());
    let reconciliation = match (options.verify, &repo) {
        (true, Some(repo)) => {
            progress("verify", &format!("reconciling against {}", repo.display()));
            reconcile::run(&session, &ledgers, &mut state, repo)
        }
        _ => reconcile::Reconciliation {
            note: Some("reconciliation skipped".to_string()),
            ..Default::default()
        },
    };
    if options.strict && reconciliation.contradictions() > 0 {
        return Err(Error::Contradicted(reconciliation.contradictions()));
    }

    // The end state, reconciled against the evidence. Runs last, after the fold
    // and after the repository check, so that what L0 presents as current really
    // is — and so that an action a later command satisfied is not handed on as
    // pending work.
    let end_state = finalize::run(&mut state, &ledgers, &reconciliation);
    for resolution in &end_state.resolutions {
        progress(
            "finalize",
            &format!(
                "{} was satisfied by `{}` (evt {})",
                resolution.id, resolution.command, resolution.evt
            ),
        );
    }
    for finding in &end_state.findings {
        progress(
            "finalize",
            &format!("{}: {}", finding.kind.label(), finding.text),
        );
    }

    let tail_rows = plan.tail.len();
    let tail_tokens: usize = rows[plan.tail.clone()].iter().map(|row| row.tokens).sum();
    let raw_tokens = session
        .source_paths
        .iter()
        .filter_map(|path| std::fs::metadata(path).ok())
        .map(|meta| meta.len() as usize / 4)
        .sum();

    let report = Report {
        sctxx: crate::VERSION,
        mode: options.mode.label(),
        llm: llm_label.clone(),
        session: format!("{}:{}", session.agent.slug(), session.id),
        agent: session.agent.slug(),
        events: session.events.len(),
        active_events: session.active.len(),
        user_turns: session.user_turns(),
        kind_counts: session.kind_counts(),
        rows: rows.len(),
        episodes: plan.episodes.len(),
        chunks: plan.chunks.len(),
        tail_rows,
        tokens: render::TokenCounts {
            raw: raw_tokens,
            masked: masked_tokens,
            artifact: 0,
            tail: tail_tokens,
        },
        fold_calls: fold_report.calls,
        premap_calls: fold_report.premap_calls,
        repair_calls: fold_report.repair_calls,
        accepted_ops: fold_report.accepted_ops,
        rejected_ops: fold_report.rejected_ops,
        semantic_state: semantic,
        triage: triage.clone(),
        guard: guard.clone(),
        fold_failed_calls: fold_report.failed_calls,
        llm_input_tokens: fold_report.input_tokens,
        llm_output_tokens: fold_report.output_tokens,
        warnings: fold_report.warnings,
        diagnostics: session.diagnostics.clone(),
        verification: reconciliation.clone(),
        elapsed_ms: started.elapsed().as_millis(),
    };

    Ok(Extraction {
        triage,
        guard,
        end_state,
        session,
        ledgers,
        rows,
        plan,
        state,
        reconciliation,
        report,
        llm_label,
    })
}

/// Where an artifact was written, and which file the receiving agent reads.
#[derive(Debug, Clone)]
pub struct Written {
    /// Every file written.
    pub paths: Vec<PathBuf>,
    /// The file a receiving agent should open.
    pub handoff: PathBuf,
    /// True when the destination was a directory and got the full set.
    pub directory: bool,
}

/// Write the artifact where the caller asked, and say what it wrote.
///
/// A directory gets the full set; a path ending in `.md` or `.json` gets just
/// that one file. Shared by the CLI and the TUI so the two cannot disagree
/// about what a destination means (FR-014).
pub fn write_destination(
    extraction: &Extraction,
    options: &ExtractOptions,
    path: &Path,
) -> Result<Written> {
    let extension = path
        .extension()
        .map(|ext| ext.to_string_lossy().into_owned());
    match extension.as_deref() {
        Some("md") => {
            write_one(path, &extraction.markdown(options))?;
            Ok(Written {
                paths: vec![path.to_path_buf()],
                handoff: path.to_path_buf(),
                directory: false,
            })
        }
        Some("json") => {
            write_one(path, &to_pretty(&extraction.json(options)))?;
            Ok(Written {
                paths: vec![path.to_path_buf()],
                handoff: path.to_path_buf(),
                directory: false,
            })
        }
        _ => {
            let written = write_all(extraction, options, path)?;
            Ok(Written {
                paths: written.paths,
                handoff: path.join("handoff.md"),
                directory: true,
            })
        }
    }
}

/// Write one file, creating its parent directory if it needs one.
fn write_one(path: &Path, content: &str) -> Result<()> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent).map_err(|source| Error::io(parent, source))?;
    }
    std::fs::write(path, content).map_err(|source| Error::io(path, source))
}

/// The files `--out <dir>` writes.
#[derive(Debug)]
pub struct WrittenFiles {
    pub paths: Vec<PathBuf>,
}

/// Write the full artifact set into `dir`.
pub fn write_all(
    extraction: &Extraction,
    options: &ExtractOptions,
    dir: &Path,
) -> Result<WrittenFiles> {
    std::fs::create_dir_all(dir).map_err(|source| Error::io(dir, source))?;
    let markdown = extraction.markdown(options);
    let mut written = Vec::new();

    let mut report = extraction.report.clone();
    report.tokens.artifact = crate::vendor::codex::truncate::approx_token_count(&markdown);

    for (name, content) in [
        ("handoff.md", markdown),
        ("handoff.json", to_pretty(&extraction.json(options))),
        ("state.json", to_pretty(&extraction.state)),
        ("ledgers.json", to_pretty(&extraction.ledgers)),
        ("report.json", to_pretty(&report)),
    ] {
        let path = dir.join(name);
        std::fs::write(&path, content).map_err(|source| Error::io(&path, source))?;
        written.push(path);
    }
    Ok(WrittenFiles { paths: written })
}

fn to_pretty<T: Serialize>(value: &T) -> String {
    serde_json::to_string_pretty(value)
        .unwrap_or_else(|error| format!("{{\"error\":\"could not serialize: {error}\"}}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modes_parse_and_label() {
        for value in ["fast", "standard", "full"] {
            assert_eq!(Mode::parse(value).expect(value).label(), value);
        }
        assert_eq!(Mode::parse("turbo").expect_err("reject").exit_code(), 2);
    }

    #[test]
    fn since_compact_prefers_a_reset_and_falls_back_to_the_earliest_window() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/codex/windowed-compaction.jsonl");
        let session = crate::adapters::parse_path(&path, 0.5).expect("the fixture parses");

        // A legacy reset (evt 1) beats a later window marker (evt 3): after a
        // real reset, everything before it is gone from the provider's view.
        assert!(session.native_compactions.len() == 2);
        assert_eq!(since_compact_boundary(&session), Some(1));

        // With window markers only, the earliest one is where the transcript is
        // still complete.
        let mut windowed_only = session.clone();
        windowed_only
            .native_compactions
            .retain(|compaction| compaction.windowed);
        assert_eq!(since_compact_boundary(&windowed_only), Some(3));

        // A session the provider never compacted keeps everything.
        let mut never_compacted = session.clone();
        never_compacted.native_compactions.clear();
        assert_eq!(since_compact_boundary(&never_compacted), None);
    }
}

#[cfg(test)]
mod semantic_tests {
    use super::*;

    fn report(calls: usize, failed: usize, accepted: usize) -> fold::FoldReport {
        fold::FoldReport {
            calls,
            failed_calls: failed,
            accepted_ops: accepted,
            ..Default::default()
        }
    }

    #[test]
    fn a_run_in_which_every_call_failed_is_unavailable_not_ordinary() {
        // The bug: 81 failed calls and an empty state rendered as a normal
        // standard handoff.
        assert_eq!(
            classify_semantic(&report(81, 81, 0)),
            SemanticState::Unavailable
        );
        assert_eq!(
            classify_semantic(&report(41, 41, 0)),
            SemanticState::Unavailable
        );
    }

    #[test]
    fn calls_that_worked_and_produced_nothing_are_degraded() {
        assert_eq!(
            classify_semantic(&report(41, 0, 0)),
            SemanticState::Degraded
        );
        // Some failures are ordinary and are not degradation.
        assert_eq!(
            classify_semantic(&report(41, 3, 0)),
            SemanticState::Degraded
        );
        assert_eq!(classify_semantic(&report(41, 3, 12)), SemanticState::Ok);
    }

    #[test]
    fn no_model_asked_for_is_its_own_state() {
        assert_eq!(
            classify_semantic(&report(0, 0, 0)),
            SemanticState::NotRequested
        );
    }

    #[test]
    fn each_state_says_something_a_reader_can_act_on() {
        assert_eq!(SemanticState::Ok.label(), "ok");
        assert!(SemanticState::Ok.notice().is_none());
        // The deterministic path says what it is missing. A reader who does not
        // know the semantic layer is absent reads a transcript digest as though
        // it were a handoff — which is what happened, and why this is no longer
        // left to the front matter alone.
        let quiet = SemanticState::NotRequested
            .notice()
            .expect("the cheap artifact says so");
        assert!(quiet.contains("No model ran"), "{quiet}");
        assert!(
            quiet.contains("--llm"),
            "and how to get the other one: {quiet}"
        );
        for state in [SemanticState::Degraded, SemanticState::Unavailable] {
            let notice = state.notice().expect("a reader must be told");
            assert!(notice.len() > 80, "{notice}");
            assert!(
                notice.contains("deterministic") || notice.contains("EMPTY"),
                "{notice}"
            );
        }
    }
}
