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

pub mod fold;
pub mod ledgers;
pub mod mask;
pub mod reconcile;
pub mod render;
pub mod segment;

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
    pub llm_input_tokens: u64,
    pub llm_output_tokens: u64,
    pub warnings: Vec<String>,
    pub diagnostics: Vec<crate::ir::Diagnostic>,
    pub verification: reconcile::Reconciliation,
    pub elapsed_ms: u128,
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
    let started = std::time::Instant::now();

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
    let mut fold_report = fold::FoldReport::default();
    let backend = crate::llm::build(&options.llm)?;
    let llm_label = match &backend {
        Some(backend) => backend.name(),
        None => "none".to_string(),
    };

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
            };
            let (folded, report) = fold::run(&input, backend.as_ref(), &fold_options)?;
            state = folded;
            fold_report = report;
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
        llm_input_tokens: fold_report.input_tokens,
        llm_output_tokens: fold_report.output_tokens,
        warnings: fold_report.warnings,
        diagnostics: session.diagnostics.clone(),
        verification: reconciliation.clone(),
        elapsed_ms: started.elapsed().as_millis(),
    };

    Ok(Extraction {
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
