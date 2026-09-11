//! S7 — rendering the handoff artifact (spec §12).
//!
//! Four layers, cheapest first, so a receiving agent can stop reading as soon
//! as it knows enough:
//!
//! * **L0 Brief** — goal, current step, next actions, hard constraints,
//!   dead ends, verify-first commands, repository drift.
//! * **L1 Items** — every active item with provenance, plus the ledgers.
//! * **L2 Recency tail** — masked rows of the end of the session.
//! * **L3 Retrieval** — the source, and the exact `sctxx expand` commands.
//!
//! Budget enforcement lives here, in Rust: items are emitted in priority order
//! until the layer budget is reached, and anything omitted is named so the
//! reader knows it exists.

use crate::ir::Session;
use crate::pipeline::fold::ops::ItemKind;
use crate::pipeline::fold::prompt::HANDOFF_PREAMBLE;
use crate::pipeline::fold::state::{FoldState, Item, ItemStatus};
use crate::pipeline::ledgers::{ErrorStatus, Ledgers};
use crate::pipeline::mask::Row;
use crate::pipeline::reconcile::Reconciliation;
use crate::vendor::codex::secrets::{RedactMode, redact};
use crate::vendor::codex::truncate::approx_token_count;
use serde::Serialize;

/// Which layers to render.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Layers {
    pub brief: bool,
    pub items: bool,
    pub tail: bool,
    pub retrieval: bool,
}

impl Default for Layers {
    fn default() -> Self {
        Self {
            brief: true,
            items: true,
            tail: true,
            retrieval: true,
        }
    }
}

impl Layers {
    /// Parse a `--layers L0,L1,L2,L3` value.
    pub fn parse(value: &str) -> crate::error::Result<Self> {
        let mut layers = Layers {
            brief: false,
            items: false,
            tail: false,
            retrieval: false,
        };
        for part in value
            .split(',')
            .map(str::trim)
            .filter(|part| !part.is_empty())
        {
            match part.to_ascii_uppercase().as_str() {
                "L0" => layers.brief = true,
                "L1" => layers.items = true,
                "L2" => layers.tail = true,
                "L3" => layers.retrieval = true,
                other => {
                    return Err(crate::error::Error::Usage(format!(
                        "unknown layer `{other}` (expected L0, L1, L2, or L3)"
                    )));
                }
            }
        }
        Ok(layers)
    }
}

/// Rendering options.
#[derive(Debug, Clone)]
pub struct RenderOptions {
    /// Total artifact budget in tokens (the tail is counted separately).
    pub budget: usize,
    pub layers: Layers,
    pub mode: &'static str,
    pub llm: String,
    /// Whether the model-written layer is present. Rendered, not assumed.
    pub semantic: crate::pipeline::SemanticState,
    /// The reconciled end state, when there is one. Owned because the
    /// artifact is rendered twice: once to measure it, once to emit it.
    pub end_state: Option<crate::pipeline::finalize::EndState>,
    pub redact: RedactMode,
    /// Tokens the masked view of the whole session costs. Known only to the
    /// pipeline, which is why it is passed in rather than recomputed here.
    pub masked_tokens: usize,
    /// Tokens this artifact costs. `Extraction::markdown` fills it from a first
    /// pass, so the header can state its own size.
    pub artifact_tokens: usize,
    /// The deterministic typed layer: standing instructions found by pattern.
    /// Rendered with its provenance and with what it cannot see, because a
    /// section labelled "binding" that was produced by a regex has to say so.
    pub triage: Option<crate::pipeline::triage::Triage>,
    /// What became of each of those constraints between extraction and here.
    pub guard: crate::pipeline::triage::GuardReport,
}

impl Default for RenderOptions {
    fn default() -> Self {
        Self {
            budget: 8_000,
            layers: Layers::default(),
            mode: "standard",
            llm: "none".to_string(),
            semantic: crate::pipeline::SemanticState::NotRequested,
            end_state: None,
            redact: RedactMode::Default,
            masked_tokens: 0,
            triage: None,
            guard: crate::pipeline::triage::GuardReport::default(),
            artifact_tokens: 0,
        }
    }
}

/// Everything the renderer needs.
#[derive(Debug)]
pub struct Artifact<'a> {
    pub session: &'a Session,
    pub ledgers: &'a Ledgers,
    pub state: &'a FoldState,
    pub reconciliation: &'a Reconciliation,
    pub tail: &'a [Row],
    pub options: &'a RenderOptions,
}

/// Token accounting for `report.json`.
#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct TokenCounts {
    pub raw: usize,
    pub masked: usize,
    pub artifact: usize,
    pub tail: usize,
}

/// The L0 budget: a brief that does not fit on a screen is not a brief.
const BRIEF_BUDGET: usize = 1_200;

/// Render `handoff.md`.
pub fn markdown(artifact: &Artifact<'_>) -> String {
    let mut out = String::new();
    out.push_str(&front_matter(artifact));
    out.push_str(&format!("\n# Handoff: {}\n\n", title(artifact)));
    out.push_str(preamble());

    let mut remaining = artifact.options.budget;
    if artifact.options.layers.brief {
        let brief = render_brief(artifact);
        remaining = remaining.saturating_sub(approx_token_count(&brief));
        out.push_str(&brief);
    }
    if artifact.options.layers.items {
        out.push_str(&render_items(artifact, remaining));
    }
    if artifact.options.layers.tail {
        out.push_str(&render_tail(artifact));
    }
    if artifact.options.layers.retrieval {
        out.push_str(&render_retrieval(artifact));
    }
    redact(&out, artifact.options.redact)
}

fn preamble() -> &'static str {
    // The preamble template's front matter is not part of the artifact.
    let body = HANDOFF_PREAMBLE;
    let after_comment = match body.find("-->") {
        Some(end) => &body[end + 3..],
        None => body,
    };
    let trimmed = after_comment.trim_start();
    match trimmed
        .strip_prefix("---")
        .and_then(|rest| rest.find("\n---").map(|end| &rest[end + 4..]))
    {
        Some(rest) => rest.trim_start(),
        None => trimmed,
    }
}

fn title(artifact: &Artifact<'_>) -> String {
    if let Some(title) = &artifact.session.meta.title {
        return title.clone();
    }
    if let Some(goal) = artifact.state.active_of(ItemKind::Goal).first() {
        return goal.text.clone();
    }
    if let Some(first) = artifact.ledgers.user_messages.first() {
        let words: Vec<&str> = first.text.split_whitespace().take(12).collect();
        return words.join(" ");
    }
    format!(
        "{} session {}",
        artifact.session.agent.label(),
        artifact.session.id
    )
}

fn front_matter(artifact: &Artifact<'_>) -> String {
    let session = artifact.session;
    let meta = &session.meta;
    let reconciliation = artifact.reconciliation;
    let counts = token_counts(artifact);
    let prompts: Vec<String> = crate::pipeline::fold::prompt::manifest()
        .into_iter()
        .map(|(id, version)| format!("{id}: {version}"))
        .collect();

    let mut out = String::from("---\n");
    out.push_str("schema: sctxx.handoff/v1\n");
    out.push_str(&format!("sctxx: {}\n", crate::VERSION));
    out.push_str(&format!(
        "source: {{agent: {}, session: {}, events: {}, active: {}, user_turns: {}",
        session.agent.slug(),
        session.id,
        session.events.len(),
        session.active.len(),
        session.user_turns()
    ));
    if let Some(started) = &meta.started_at {
        out.push_str(&format!(", started: {started}"));
    }
    if let Some(ended) = &meta.ended_at {
        out.push_str(&format!(", ended: {ended}"));
    }
    if let Some(cwd) = meta.cwd.as_deref() {
        out.push_str(&format!(", cwd: {}", posix(cwd)));
    }
    if let Some(branch) = &meta.git_branch {
        out.push_str(&format!(", branch: {branch}"));
    }
    out.push_str("}\n");
    out.push_str(&format!("mode: {}\n", artifact.options.mode));
    out.push_str(&format!("llm: {}\n", artifact.options.llm));
    out.push_str(&format!(
        "semantic: {}\n",
        artifact.options.semantic.label()
    ));
    if let Some(triage) = artifact.options.triage.as_ref() {
        let guard = &artifact.options.guard;
        out.push_str(&format!(
            "triage: {{constraints: {}, preserved: {}, restored: {}, missing: {}, \
             messages: {}, above_cap: {}}}\n",
            guard.found,
            guard.preserved,
            guard.restored,
            guard.missing.len(),
            triage.messages,
            triage.dropped,
        ));
    }
    out.push_str(&format!("prompts: {{{}}}\n", prompts.join(", ")));
    out.push_str(&format!(
        "verification: {{repo: {}, head: {}, commits_since_session: {}, stale: {}, contradicted: {}}}\n",
        reconciliation.repo.as_deref().map(posix).unwrap_or_else(|| "none".to_string()),
        reconciliation.head.as_deref().map(short_sha).unwrap_or_else(|| "none".to_string()),
        reconciliation.commits_since_session.len(),
        reconciliation.stale(),
        reconciliation.contradictions()
    ));
    out.push_str(&format!(
        "tokens: {{raw: {}, masked: {}, artifact: {}, tail: {}}}\n",
        counts.raw, counts.masked, counts.artifact, counts.tail
    ));
    out.push_str("---\n");
    out
}

fn short_sha(sha: &str) -> String {
    sha.chars().take(7).collect()
}

fn posix(path: &std::path::Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// Below this artifact budget, the ledger is short enough that a workset would
/// only duplicate it.
const WORKSET_MIN_BUDGET: usize = 1_000;

/// L0: what the next agent must know before touching anything.
fn render_brief(artifact: &Artifact<'_>) -> String {
    let state = artifact.state;
    let ledgers = artifact.ledgers;
    let mut out = String::from("\n## L0 · Brief\n\n");

    // `--budget` bounds the artifact and L0 is part of it. Three blocks are
    // charged to it *first* and never refused, because an artifact that fits its
    // budget by deleting them is not smaller, it is wrong:
    //
    //   the notice that no model ran, the evidence that contradicts the handoff,
    //   and the user's own standing instructions.
    //
    // They spend from the top, so the optional content — what the session did,
    // where the work was, the arc, the derived next actions — is what a tight
    // budget trims. If the mandatory blocks alone exceed the budget the artifact
    // exceeds it, visibly: the alternative is a briefing that does not mention
    // that its semantic layer is missing.
    let mut budget = BRIEF_BUDGET.min(artifact.options.budget);

    // Before any content, because a reader who acts on an empty state as though
    // it were a full one is worse off than one who was told.
    if let Some(notice) = artifact.options.semantic.notice() {
        let mut block = String::new();
        for line in notice.lines() {
            block.push_str(&format!("> {line}\n"));
        }
        block.push('\n');
        spend_priority(&mut out, block, &mut budget);
    }

    // What the evidence says that the state does not, before the goal: a reader
    // who acts on a stale goal wastes the whole session.
    if let Some(end) = artifact.options.end_state.as_ref()
        && !end.findings.is_empty()
    {
        let mut block =
            String::from("**Before you act — the evidence contradicts this handoff:**\n");
        for finding in &end.findings {
            block.push_str(&format!("- *{}*: {}\n", finding.kind.label(), finding.text));
        }
        block.push('\n');
        spend_priority(&mut out, block, &mut budget);
    }

    // `--budget` bounds the artifact and L0 is part of it, so the brief takes the
    // smaller of its own ceiling and what the caller allowed. Before this, a
    // `--budget 400` run still emitted a 1,200-token brief.
    let push = |out: &mut String, text: String, budget: &mut usize| {
        let cost = approx_token_count(&text);
        if cost > *budget {
            return;
        }
        *budget -= cost;
        out.push_str(&text);
    };

    // The first request is provenance, not the active objective: on a ten-day
    // session "onboard yourself to this project" says where the work started,
    // not what it is now. When the fold has evolved a goal, both are shown and
    // labelled for what they are.
    let goals = state.active_of(ItemKind::Goal);
    let folded_goal = goals.first();
    if let Some(first) = ledgers.user_messages.first() {
        let label = if folded_goal.is_some() {
            "**Original request** (where the work started)"
        } else {
            "**Goal** (from the first user message, not model-inferred)"
        };
        push(
            &mut out,
            format!(
                "{label}: {} [evt {}]\n\n",
                one_line(&first.text, 300),
                first.evt
            ),
            &mut budget,
        );
    }
    if let Some(goal) = folded_goal {
        push(
            &mut out,
            format!(
                "**Active goal** ({}): {} {}\n\n",
                goal.id,
                goal.text,
                goal.provenance()
            ),
            &mut budget,
        );
    }

    // Hard constraints come before everything except the contradictions and the
    // goal, and they are never dropped whole. Two separate defects lived here:
    // the section was empty whenever no model ran, because only the fold could
    // create a constraint; and when it was not empty, a block that overran the
    // remaining budget was discarded in its entirety by `push` above, silently,
    // while the artifact's own preamble went on telling its reader to treat the
    // section as binding.
    render_constraints(&mut out, artifact, &mut budget);

    // What the session *did*. All of this is already in the ledgers and none of it
    // was reaching L0: a reader was told which files had gone missing since and
    // nothing about what the ten days were spent on.
    push(&mut out, session_work(artifact), &mut budget);
    push(&mut out, the_arc(artifact), &mut budget);

    // The last thing the human asked is the sharpest statement of intent the
    // transcript contains, and it costs nothing to compute.
    if ledgers.user_messages.len() > 1
        && let Some(last) = ledgers.user_messages.last()
    {
        push(
            &mut out,
            format!(
                "**Last user request**: {} [evt {}]\n\n",
                one_line(&last.text, 300),
                last.evt
            ),
            &mut budget,
        );
    }

    // The provider's own compaction summary is often the highest-value semantic
    // object in a long session, and it is also the least trustworthy: it is the
    // provider's account of what happened, written by a model, frozen at a point
    // in time. Shown in L0 with its pointer and that label, so a reader can use
    // it and check it instead of never seeing it.
    if let Some(summary) = ledgers.prior_summaries.last() {
        push(
            &mut out,
            format!(
                "**Provider summary** (low trust \u{2014} written by a model, not evidence; verify \
                 against the repository and the recency tail) [evt {}]: {}\n\n",
                summary.evt,
                one_line(&summary.text, 280)
            ),
            &mut budget,
        );
    }

    match state.active_of(ItemKind::CurrentStep).first() {
        Some(step) => push(
            &mut out,
            format!(
                "**Current step** ({}): {} {}\n\n",
                step.id,
                step.text,
                step.provenance()
            ),
            &mut budget,
        ),
        // No folded step: derive one from the ledger rather than leave the
        // reader without an end state, and say where it came from.
        None => {
            if let Some(derived) = artifact
                .options
                .end_state
                .as_ref()
                .and_then(|end| end.derived_step.as_ref())
            {
                push(
                    &mut out,
                    format!(
                        "**Current step** — {} [evt {}]\n\n",
                        derived.text, derived.evt
                    ),
                    &mut budget,
                );
            }
        }
    }

    let next_actions = state.active_of(ItemKind::NextAction);
    if !next_actions.is_empty() {
        let mut block = String::from("**Next actions**\n");
        let _ = &block;
        for (index, item) in next_actions.iter().enumerate() {
            block.push_str(&format!(
                "{}. ({}) {} {}\n",
                index + 1,
                item.id,
                item.text,
                item.provenance()
            ));
        }
        block.push('\n');
        push(&mut out, block, &mut budget);
    } else if let Some(block) = ledger_next_actions(artifact) {
        // No fold ran, so derive next actions from the ledgers and label them
        // as derived. A deterministic artifact still has to be actionable.
        push(&mut out, block, &mut budget);
    }

    // Work the session already finished. Without this a receiving agent redoes
    // it, which is the failure the end-state pass exists to prevent.
    if let Some(end) = artifact.options.end_state.as_ref()
        && !end.resolutions.is_empty()
    {
        let mut block = String::from("**Already done** (a later command satisfied these)\n");
        for resolution in &end.resolutions {
            block.push_str(&format!(
                "- {} \u{2014} `{}` succeeded [evt {}]\n",
                one_line(&resolution.text, 160),
                resolution.command,
                resolution.evt
            ));
        }
        block.push('\n');
        push(&mut out, block, &mut budget);
    }

    let dead_ends = state.active_of(ItemKind::DeadEnd);
    if !dead_ends.is_empty() {
        let mut block = String::from("**Don't retry**\n");
        for item in dead_ends.iter().take(5) {
            let why = item
                .why
                .as_deref()
                .map(|why| format!(" — {why}"))
                .unwrap_or_default();
            block.push_str(&format!(
                "- ({}) {}{} {}\n",
                item.id,
                item.text,
                why,
                item.provenance()
            ));
        }
        block.push('\n');
        push(&mut out, block, &mut budget);
    }

    // Verify first: the commands that tell the next agent where reality is.
    let mut verify: Vec<String> = vec!["git status".into(), "git log --oneline -5".into()];
    for command in ledgers.last_command_status().iter().take(3) {
        if matches!(
            command.category,
            crate::pipeline::ledgers::CmdCategory::Test
                | crate::pipeline::ledgers::CmdCategory::Build
                | crate::pipeline::ledgers::CmdCategory::Lint
        ) {
            verify.push(command.normalized.clone());
        }
    }
    verify.dedup();
    push(
        &mut out,
        format!(
            "**Verify first**\n{}\n",
            verify
                .iter()
                .map(|command| format!("- `{command}`\n"))
                .collect::<String>()
        ),
        &mut budget,
    );

    if artifact.reconciliation.repo_moved() {
        // Counts, not a file list. Ten paths saying "changed after the session
        // ended" is a diff a reader did not ask for and cannot use; the paths are
        // in the workset in L1, where a reader goes to act.
        push(
            &mut out,
            format!(
                "**Since this session** {} new commit(s) and {} changed file(s) — see the workset in L1.\n\n",
                artifact.reconciliation.commits_since_session.len(),
                artifact.reconciliation.changed_since_session.len()
            ),
            &mut budget,
        );
    }
    if artifact.reconciliation.contradictions() > 0 {
        push(
            &mut out,
            format!(
                "> [!WARNING]\n> {} claim(s) in this artifact contradict the current repository: {}.\n\n",
                artifact.reconciliation.contradictions(),
                artifact.reconciliation.contradicted_files.join(", ")
            ),
            &mut budget,
        );
    }
    out
}

/// Next actions a reviewer could read straight off the ledgers: the plan item
/// that was in progress, and any command whose last run failed.
#[cfg(test)]
mod action_quality_tests {
    use super::*;

    #[test]
    fn a_script_is_not_a_next_action() {
        // This is the 2,808-character line that was 42% of a real L0.
        let heredoc =
            "cat >> docs/DEVELOPMENT-LOG.md << 'EOF' ## 2026-09-11 - fix: something very long";
        assert!(
            !is_runnable(heredoc),
            "a heredoc writes a file; it is not an action"
        );
        assert!(!is_runnable("first line\nsecond line"));
        assert!(!is_runnable(&"x".repeat(300)));
        assert!(is_runnable("pnpm vitest run packages/ext-engine"));
    }

    #[test]
    fn the_recency_window_keeps_the_end_of_a_long_session() {
        // A failure at evt 22,372 of 103,757 is not something to do next.
        let window = recent_window(103_757);
        assert!(window > 98_000, "{window}");
        assert!(22_372 < window);
        assert!(103_000 > window);
        // A short session still has a floor, so the window is never empty.
        assert_eq!(recent_window(100), 0);
    }

    #[test]
    fn whitespace_runs_are_collapsed_when_quoting() {
        // A stack trace quoted with its indentation reads as damage.
        let quoted = one_line(
            "Error: not registered     at Object.mount\n  next line",
            200,
        );
        assert_eq!(quoted, "Error: not registered at Object.mount next line");
    }
}

/// What the session spent itself on, from the ledgers alone.
///
/// A receiving agent's first question is "what is this work", and the ledgers
/// answer it: which parts of the tree were touched and how hard, and what the
/// commits said. On the session this was written from that is
/// `acryl-tui/src`, `acryl-desktop/src`, `acryl-harness-runtime/src` and 46
/// commits naming the engine-swap work — none of which reached L0 before.
fn session_work(artifact: &Artifact<'_>) -> String {
    let ledgers = artifact.ledgers;
    let mut out = String::new();

    // Where the work was. Directories, not files: 832 paths say nothing, and
    // eight areas name the subsystems.
    let mut areas: std::collections::BTreeMap<String, u32> = std::collections::BTreeMap::new();
    for file in &ledgers.files {
        let area = subsystem_of(&file.path);
        *areas.entry(area).or_insert(0) += file.edits + file.reads;
    }
    let mut ranked: Vec<(String, u32)> = areas.into_iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    // Anything outside the project is the environment, not the work.
    ranked.retain(|(area, _)| area != OUTSIDE);
    if !ranked.is_empty() {
        out.push_str("**Where the work was**\n");
        for (area, weight) in ranked.iter().take(5) {
            out.push_str(&format!("- `{area}` — {weight} touch(es)\n"));
        }
        out.push('\n');
    }

    // What it committed. A commit subject is the session's own summary of a unit
    // of work, written by the agent, and it costs nothing to read.
    let commits = &ledgers.git.commits;
    if !commits.is_empty() {
        out.push_str(&format!(
            "**What it committed** ({} total)\n",
            commits.len()
        ));
        for commit in commits.iter().rev().take(5) {
            out.push_str(&format!(
                "- `{}` {}\n",
                short_sha(&commit.sha),
                one_line(&commit.subject, 110)
            ));
        }
        out.push('\n');
    }
    out
}

/// The arc of what the human asked for, sampled across the session.
///
/// The first message says where the work started and the last says where it
/// stopped, and neither says what happened in between. On a 274-turn session the
/// asks *are* the trajectory: onboarding, then loader errors, then naming, then
/// "the plugin is active but I do not see it anywhere". Sampling them costs a
/// few hundred tokens and is the difference between a snapshot and a story.
fn the_arc(artifact: &Artifact<'_>) -> String {
    let turns = &artifact.ledgers.user_messages;
    // Below this there is no arc to sample; the first and last request already
    // bracket the whole conversation.
    const MIN_TURNS: usize = 8;
    const SHOWN: usize = 5;
    if turns.len() < MIN_TURNS {
        return String::new();
    }

    // The first and last are shown elsewhere, so the middle is what is sampled.
    let inner = &turns[1..turns.len() - 1];
    let step = (inner.len() / SHOWN).max(1);
    let mut out = String::from("**The arc** (every ~");
    out.push_str(&format!("{step}th of {} asks)\n", turns.len()));
    for (index, turn) in inner.iter().enumerate() {
        if index % step != 0 || out.matches('\n').count() > SHOWN + 1 {
            continue;
        }
        out.push_str(&format!(
            "- evt {}: {}\n",
            turn.evt,
            one_line(&turn.text, 120)
        ));
    }
    out.push('\n');
    out
}

/// The part of a path that names a subsystem.
///
/// Repository-relative, two components deep: `acryl-tui/src`, `specs/028-…`.
/// Everything outside the project is one bucket, because npm cache paths and
/// log files are what the session read, not what it built.
fn subsystem_of(path: &str) -> String {
    // A path the session read from npm's cache or a log directory is the
    // environment, not the work, and it does not belong in a workstream list.
    let Some(after_repo) = path.split("/acryl/").nth(1) else {
        return OUTSIDE.to_string();
    };
    let after_repo = after_repo.trim_start_matches('/');
    let mut parts = after_repo.split('/').filter(|p| !p.is_empty());
    match (parts.next(), parts.next()) {
        (Some(first), Some(second)) if second.contains('.') => first.to_string(),
        (Some(first), Some(second)) => format!("{first}/{second}"),
        (Some(first), None) => first.to_string(),
        _ => OUTSIDE.to_string(),
    }
}

/// Activity that belongs to the machine rather than to the project.
const OUTSIDE: &str = "outside the project";

/// The first event that counts as "near the end of the session".
///
/// Five percent, with a floor, so a short session still has a window.
fn recent_window(events: usize) -> u32 {
    let five_percent = (events / 20) as u32;
    (events as u32).saturating_sub(five_percent.max(500))
}

/// Whether a recorded command could be handed back as something to re-run.
///
/// One line, and short enough to read. A multi-line command is a script, and a
/// very long one is usually a heredoc that wrote a file — both belong in the
/// ledger, neither is a next action.
fn is_runnable(command: &str) -> bool {
    !command.trim().is_empty()
        && !command.contains('\n')
        // A heredoc marker makes it a script even when the ledger collapsed the
        // newlines, which is how the 2,800-character one arrived.
        && !command.contains("<<")
        && command.chars().count() <= 200
}

fn ledger_next_actions(artifact: &Artifact<'_>) -> Option<String> {
    let ledgers = artifact.ledgers;
    let mut actions: Vec<String> = Vec::new();

    // Six plan items, six files, two errors: the workset is a starting point, and
    // everything it leaves out is in the full ledger immediately below it.
    if let Some(plan) = &ledgers.plan {
        for item in plan
            .items
            .iter()
            .filter(|item| matches!(item.status.as_str(), "in_progress" | "active" | "pending"))
        {
            actions.push(format!(
                "{} (plan item, {}) [evt {}]",
                item.text, item.status, plan.evt
            ));
        }
    }
    // A failed *command* is a next action only when it is a command. On a real
    // session the newest failures are heredocs — a 2,800-character `cat >> … EOF`
    // script — and quoting one at full length costs more of L0 than everything
    // else in it put together while telling a reader nothing they can re-run.
    let mut seen: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    // "The last run of this command failed" is only a next action if that run was
    // *near the end*. On a 274-turn session the newest failure of `npm view` was
    // 80,000 events before the session stopped, and offering it as something to do
    // next sends a reader into the middle of a week-old session.
    let recent_after = recent_window(artifact.session.events.len());
    let mut failures: Vec<&crate::pipeline::ledgers::CommandRecord> = ledgers
        .last_command_status()
        .into_iter()
        .filter(|command| command.failed())
        .filter(|command| command.evt >= recent_after)
        .filter(|command| is_runnable(&command.normalized))
        .collect();
    // A failing test or build is a next action; a failing `grep` is a dead end
    // the session already walked past. Rank by what the failure was in.
    failures.sort_by_key(|command| {
        (
            match command.category {
                crate::pipeline::ledgers::CmdCategory::Test => 0,
                crate::pipeline::ledgers::CmdCategory::Build => 1,
                crate::pipeline::ledgers::CmdCategory::Lint => 2,
                crate::pipeline::ledgers::CmdCategory::Run => 3,
                crate::pipeline::ledgers::CmdCategory::PackageManager => 4,
                _ => 5,
            },
            std::cmp::Reverse(command.evt),
        )
    });
    for command in failures
        .into_iter()
        .filter(|command| seen.insert(command.normalized.clone()))
        .take(3)
    {
        actions.push(format!(
            "re-run `{}` \u{2014} its last run FAILED [evt {}]",
            one_line(&command.normalized, 90),
            command.evt
        ));
    }
    for error in ledgers.unresolved_errors().iter().take(2) {
        actions.push(format!(
            "resolve `{}` (\u{d7}{}) \u{2014} {} [evt {}\u{2013}{}]",
            error.sig,
            error.occurrences,
            one_line(&error.example, 140),
            error.first_evt,
            error.last_evt
        ));
    }
    if actions.is_empty() {
        return None;
    }

    let mut block =
        String::from("**Next actions** (derived from the ledgers, not model-inferred)\n");
    for (index, action) in actions.iter().take(4).enumerate() {
        block.push_str(&format!("{}. {action}\n", index + 1));
    }
    block.push('\n');
    Some(block)
}

/// L1: every active item with provenance, then the ledgers.
fn render_items(artifact: &Artifact<'_>, budget: usize) -> String {
    let mut out = String::from("\n## L1 · Items\n");
    let mut remaining = budget;
    let mut omitted: Vec<&str> = Vec::new();

    for kind in ItemKind::PRIORITY {
        let items = artifact.state.active_of(kind);
        if items.is_empty() {
            continue;
        }
        let mut block = format!("\n### {}\n", kind.heading());
        for item in items {
            let line = item_line(item);
            let cost = approx_token_count(&line);
            if cost > remaining {
                omitted.push(&item.id);
                continue;
            }
            remaining -= cost;
            block.push_str(&line);
        }
        out.push_str(&block);
    }

    // Only reversals a reader must know about: a superseded next action is
    // just bookkeeping, a superseded decision is a warning.
    let superseded: Vec<&Item> = artifact
        .state
        .superseded()
        .into_iter()
        .filter(|item| {
            matches!(
                item.kind,
                ItemKind::Decision | ItemKind::Constraint | ItemKind::Goal
            )
        })
        .collect();
    if !superseded.is_empty() {
        out.push_str("\n### Earlier decisions later reversed\n");
        for item in &superseded {
            let by = match &item.status {
                ItemStatus::Superseded { by } => by.as_str(),
                _ => "?",
            };
            out.push_str(&format!(
                "- ~~{}~~ {} — superseded by {by} {}\n",
                item.id,
                item.text,
                item.provenance()
            ));
        }
    }

    out.push_str(&render_ledgers(artifact));

    if !omitted.is_empty() {
        // The ids are capped, and this is why. Listing every omitted id makes
        // the layer's size linear in the number of items the budget just
        // excluded, so `--budget` stops bounding the artifact: at a page of 20
        // ids the footer is longer than the content it replaced. A bounded
        // pointer with a count is the honest form — the reader is told how many
        // there are and where the rest live.
        const OMITTED_STUBS: usize = 12;
        let shown: Vec<&str> = omitted.iter().take(OMITTED_STUBS).copied().collect();
        let more = omitted.len().saturating_sub(shown.len());
        let tail = if more > 0 {
            format!(" and {more} more")
        } else {
            String::new()
        };
        out.push_str(&format!(
            "\n_{} item(s) omitted for the artifact budget: {}{}. See `sctxx show state.json`._\n",
            omitted.len(),
            shown.join(", "),
            tail
        ));
    }
    out
}

fn item_line(item: &Item) -> String {
    let mut line = format!("- {} · {}", item.id, item.confidence.label());
    if item.verified != crate::pipeline::fold::state::Verification::Unchecked {
        line.push_str(&format!(" · {}", item.verified.label()));
    }
    line.push_str(&format!(" — {}", item.text));
    if let Some(quote) = &item.quote {
        line.push_str(&format!(" · quote: \"{quote}\""));
    }
    if let Some(why) = &item.why {
        line.push_str(&format!(" · *why*: {why}"));
    }
    if !item.rejected.is_empty() {
        line.push_str(&format!(" · *rejected*: {}", item.rejected.join(", ")));
    }
    line.push_str(&format!(" {}\n", item.provenance()));
    line
}

/// The active workset: what a receiving agent needs to touch first.
///
/// The ledgers are forensic — every file, every error, every command — and a
/// history of 548 touched files is not the thing an agent needs before it starts.
/// This is the small, current subset: what the work *is*, what is open right now,
/// and what to run. The full ledgers follow it, and stay retrievable.
/// Tokens the constraints block may spend before it starts summarising itself.
///
/// The block is capped rather than deferring to the whole-brief budget: a brief
/// that silently loses its binding instructions is the failure this whole layer
/// exists to prevent, so an overflow is made visible instead (`… and N more`).
const CONSTRAINT_BUDGET: usize = 700;

/// Render the deterministic hard lane, and say what it is.
fn render_constraints(out: &mut String, artifact: &Artifact<'_>, budget: &mut usize) {
    let constraints = artifact.state.active_of(ItemKind::Constraint);
    if constraints.is_empty() {
        return;
    }
    let mut block = String::from("**Hard constraints** (standing instructions, quoted verbatim)\n");
    let mut cost = approx_token_count(&block);
    let mut shown = 0usize;
    for item in &constraints {
        let quote = item.quote.as_deref().unwrap_or(&item.text);
        let line = format!("- ({}) \"{}\" {}\n", item.id, quote, item.provenance());
        let line_cost = approx_token_count(&line);
        if shown > 0 && cost + line_cost > CONSTRAINT_BUDGET.min(*budget) {
            break;
        }
        cost += line_cost;
        block.push_str(&line);
        shown += 1;
    }
    if shown < constraints.len() {
        // Named, not silently omitted: the reader can reach them in L1 and the
        // count tells them how much of the section they are not seeing.
        block.push_str(&format!(
            "- \u{2026} and {} more (see L1 \u{b7} Active workset)\n",
            constraints.len() - shown
        ));
    }
    block.push('\n');

    // What this section cannot see. A regex found these; a rule stated
    // declaratively — "the schema is frozen until the migration lands" — is not
    // here, and a reader who does not know that reads silence as permission.
    if artifact.options.semantic == crate::pipeline::SemanticState::NotRequested {
        block.push_str(
            "> Found by pattern, not by model: no model ran, so a rule stated declaratively is \
             absent rather than absent-minded. Treat a missing rule as unknown, not as permission.\
             \n",
        );
    }
    let restored = artifact.options.guard.restored;
    if restored > 0 {
        block.push_str(&format!(
            "> {restored} constraint(s) were dropped by the semantic pass and put back here.\n"
        ));
    }
    if !artifact.options.guard.is_clean() {
        block.push_str(&format!(
            "> {} constraint(s) could not be restored; see report.json.\n",
            artifact.options.guard.missing.len()
        ));
    }
    block.push('\n');

    spend_priority(out, block, budget);
}

/// Spend from the budget without ever refusing.
///
/// Everything else in the brief goes through `push`, which drops a block that
/// does not fit. These blocks are the ones a reader cannot do without, so they
/// take the space and leave the remainder to the optional content.
fn spend_priority(out: &mut String, text: String, budget: &mut usize) {
    *budget = budget.saturating_sub(approx_token_count(&text));
    out.push_str(&text);
}

fn render_workset(artifact: &Artifact<'_>) -> String {
    // A summary only helps when there is something to summarise. Under a very
    // small budget the full ledger below is already short, so the workset would
    // be a second copy of it — and `--budget` is a promise about the artifact's
    // size, not a suggestion.
    if artifact.options.budget < WORKSET_MIN_BUDGET {
        return String::new();
    }
    let ledgers = artifact.ledgers;
    let reconciliation = artifact.reconciliation;
    let mut out = String::from("\n### Active workset\n\n");

    // The plan the agent was working to, which is the closest thing to a spec
    // pointer the transcript carries.
    // Six plan items, six files, two errors: the workset is a starting point, and
    // everything it leaves out is in the full ledger immediately below it.
    if let Some(plan) = &ledgers.plan {
        let open: Vec<_> = plan
            .items
            .iter()
            .filter(|item| !matches!(item.status.as_str(), "completed" | "cancelled"))
            .collect();
        if !open.is_empty() {
            out.push_str(&format!("**Plan at evt {}**\n", plan.evt));
            for item in open.iter().take(5) {
                // The plan is one snapshot at `plan.evt`; individual items do
                // not carry their own pointer.
                out.push_str(&format!(
                    "- {} ({}) [evt {}]\n",
                    one_line(&item.text, 160),
                    item.status,
                    plan.evt
                ));
            }
            out.push('\n');
        }
    }

    // What changed most recently, not everything that ever changed.
    let mut recent: Vec<&crate::pipeline::ledgers::FileRecord> =
        ledgers.files.iter().filter(|file| !file.deleted).collect();
    recent.sort_by_key(|file| std::cmp::Reverse(file.last_evt));
    if !recent.is_empty() {
        out.push_str("**Most recently touched**\n");
        for file in recent.iter().take(6) {
            out.push_str(&format!(
                "- `{}` — {} edit(s), last evt {} [evt {}]\n",
                posix(std::path::Path::new(&file.path)),
                file.edits,
                file.last_evt,
                file.last_evt
            ));
        }
        if ledgers.files.len() > recent.len().min(6) {
            out.push_str(&format!(
                "\n_{} file(s) in total; the full ledger is below._\n",
                ledgers.files.len()
            ));
        }
        out.push('\n');
    }

    // The most recent run of each of these is what "is it green?" means.
    for (label, category) in [
        ("Latest test", crate::pipeline::ledgers::CmdCategory::Test),
        ("Latest build", crate::pipeline::ledgers::CmdCategory::Build),
        ("Latest lint", crate::pipeline::ledgers::CmdCategory::Lint),
    ] {
        let latest = ledgers
            .commands
            .iter()
            .filter(|command| command.category == category)
            .max_by_key(|command| command.evt);
        if let Some(command) = latest {
            out.push_str(&format!(
                "**{label}**: `{}` — {} [evt {}]\n",
                one_line(&command.normalized, 120),
                command.status(),
                command.evt
            ));
        }
    }

    // What is still broken, which is the one ledger section that is *current*
    // rather than historical.
    let unresolved = ledgers.unresolved_errors();
    if unresolved.is_empty() {
        out.push_str("**Unresolved errors**: none\n");
    } else {
        out.push_str(&format!("**Unresolved errors**: {}\n", unresolved.len()));
        for error in unresolved.iter().take(2) {
            out.push_str(&format!(
                "- {} (×{}) [evt {}]\n",
                one_line(&error.example, 110),
                error.occurrences,
                error.last_evt
            ));
        }
    }

    // The repository as it is now, which the session could not know.
    if !reconciliation.changed_since_session.is_empty() {
        out.push_str(&format!(
            "**Changed since the session ended**: {} file(s)\n",
            reconciliation.changed_since_session.len()
        ));
    }
    if reconciliation.uncommitted_changes > 0 {
        out.push_str(&format!(
            "**Uncommitted right now**: {}\n",
            reconciliation.uncommitted_changes
        ));
    }
    let commits = &ledgers.git.commits;
    if !commits.is_empty() {
        out.push_str("**Commits this session made**\n");
        for commit in commits.iter().rev().take(3) {
            out.push_str(&format!(
                "- `{}` {} [evt {}]\n",
                short_sha(&commit.sha),
                one_line(&commit.subject, 100),
                commit.evt
            ));
        }
    }
    if !reconciliation.commits_since_session.is_empty() {
        out.push_str(&format!(
            "**Commits after the session**: {}\n",
            reconciliation.commits_since_session.len()
        ));
    }
    out.push('\n');
    out
}

/// The deterministic ledgers: true whether or not a model ran.
fn render_ledgers(artifact: &Artifact<'_>) -> String {
    let ledgers = artifact.ledgers;
    let mut out = render_workset(artifact);

    let edited = ledgers.edited_files();
    if !edited.is_empty() {
        out.push_str(
            "\n#### Files touched (the full ledger)\n\n| path | ops | last evt | status |\n|---|---|---|---|\n",
        );
        for file in edited.iter().take(40) {
            let mut status = if file.deleted { "deleted" } else { "exists" }.to_string();
            if artifact.reconciliation.missing_files.contains(&file.path) {
                status = "missing now".to_string();
            } else if artifact
                .reconciliation
                .changed_since_session
                .contains(&file.path)
            {
                status = "changed since session".to_string();
            } else if artifact
                .reconciliation
                .contradicted_files
                .contains(&file.path)
            {
                status = "still exists (contradicts)".to_string();
            }
            if file.inferred {
                status.push_str(" · inferred");
            }
            out.push_str(&format!(
                "| `{}` | edit×{} read×{} | {} | {} |\n",
                file.path, file.edits, file.reads, file.last_evt, status
            ));
        }
        if edited.len() > 40 {
            out.push_str(&format!(
                "\n_{} more files in `ledgers.json`._\n",
                edited.len() - 40
            ));
        }
    }

    let statuses = ledgers.last_command_status();
    if !statuses.is_empty() {
        out.push_str("\n### Last known command status\n");
        for command in statuses.iter().take(12) {
            out.push_str(&format!(
                "- `{}` → **{}**{} [evt {}]\n",
                command.normalized,
                command.status(),
                command
                    .exit_code
                    .map(|code| format!(" (exit {code})"))
                    .unwrap_or_default(),
                command.evt
            ));
        }
    }

    let unresolved = ledgers.unresolved_errors();
    if !unresolved.is_empty() {
        out.push_str("\n### Unresolved errors\n");
        for error in unresolved.iter().take(10) {
            out.push_str(&format!(
                "- `{}` ×{} — {} [evt {}–{}]\n",
                error.sig,
                error.occurrences,
                one_line(&error.example, 200),
                error.first_evt,
                error.last_evt
            ));
        }
    }

    let resolved = ledgers
        .errors
        .iter()
        .filter(|error| matches!(error.status, ErrorStatus::Resolved { .. }))
        .count();
    if resolved > 0 {
        out.push_str(&format!(
            "\n_{resolved} earlier error signature(s) were resolved._\n"
        ));
    }

    // Six plan items, six files, two errors: the workset is a starting point, and
    // everything it leaves out is in the full ledger immediately below it.
    if let Some(plan) = &ledgers.plan {
        out.push_str(&format!(
            "\n### Plan as last published [evt {}]\n",
            plan.evt
        ));
        for item in &plan.items {
            let mark = match item.status.as_str() {
                "completed" | "done" => "x",
                _ => " ",
            };
            out.push_str(&format!("- [{mark}] {} ({})\n", item.text, item.status));
        }
    }

    if !ledgers.git.commits.is_empty() || !ledgers.git.pull_requests.is_empty() {
        out.push_str("\n### Git\n");
        for commit in ledgers.git.commits.iter().take(15) {
            let missing = artifact
                .reconciliation
                .missing_commits
                .contains(&commit.sha);
            out.push_str(&format!(
                "- {} {}{}\n",
                commit.sha,
                commit.subject,
                if missing {
                    " — no longer in this repository (rebased or squashed?)"
                } else {
                    ""
                }
            ));
        }
        for url in &ledgers.git.pull_requests {
            out.push_str(&format!("- pull request: {url}\n"));
        }
        if ledgers.git.pushed {
            out.push_str("- the session pushed at least once\n");
        }
    }

    if !ledgers.prior_summaries.is_empty() {
        out.push_str(&format!(
            "\n_This session contained {} provider compaction summary/summaries; they are low-trust \
             and were used only as seeds._\n",
            ledgers.prior_summaries.len()
        ));
    }
    out
}

/// L2: the end of the session, near-verbatim.
fn render_tail(artifact: &Artifact<'_>) -> String {
    if artifact.tail.is_empty() {
        return String::new();
    }
    let first = artifact.tail.first().map(|row| row.evt).unwrap_or(0);
    let last = artifact.tail.last().map(|row| row.evt).unwrap_or(0);

    let body: String = artifact
        .tail
        .iter()
        .map(|row| format!("{}  (evt {})\n", row.text, row.evt))
        .collect();

    // Transcript text routinely contains code fences and markdown headings.
    // A fixed ```-fence would let that content escape the block and become
    // artifact structure, so the fence is always longer than anything inside.
    let fence = fence_for(&body);
    format!(
        "\n## L2 · Recent activity (masked, evt {first}–{last})\n\n{fence}text\n{body}{fence}\n"
    )
}

/// A backtick fence guaranteed to be longer than any run of backticks in
/// `content`, so the content cannot terminate it early.
fn fence_for(content: &str) -> String {
    let mut longest = 0usize;
    let mut current = 0usize;
    for character in content.chars() {
        if character == '`' {
            current += 1;
            longest = longest.max(current);
        } else {
            current = 0;
        }
    }
    "`".repeat(longest.max(2) + 1)
}

/// L3: how to get back to the raw events.
fn render_retrieval(artifact: &Artifact<'_>) -> String {
    let session = artifact.session;
    let reference = format!("{}:{}", session.agent.slug(), session.id);
    let mut out = String::from("\n## L3 · Retrieval\n\n");
    out.push_str(&format!(
        "Source: {} session `{}`\n",
        session.agent.label(),
        session.id
    ));
    for path in &session.source_paths {
        out.push_str(&format!("- `{}`\n", posix(path)));
    }
    out.push_str(&format!(
        "\nExpand any pointer:\n```sh\nsctxx expand {reference} <a>..<b> --context 3\n"
    ));

    // Concrete commands for the items most likely to need detail.
    let mut examples: Vec<String> = Vec::new();
    for kind in [
        ItemKind::CurrentStep,
        ItemKind::NextAction,
        ItemKind::DeadEnd,
    ] {
        for item in artifact.state.active_of(kind) {
            if let Some(span) = item.span() {
                examples.push(format!(
                    "sctxx expand {reference} {}..{}   # {}\n",
                    span.start, span.end, item.id
                ));
            }
        }
    }
    for example in examples.iter().take(4) {
        out.push_str(example);
    }
    out.push_str("```\n");
    out.push_str(&format!(
        "\nFull state including superseded and dropped items: `state.json`. \
         All deterministic ledgers: `ledgers.json`. Run details: `report.json`.\n\
         \nSession source hash: `{}`\n",
        short_sha(&session.source_hash)
    ));
    if !session.diagnostics.is_empty() {
        out.push_str(&format!(
            "\n_{} adapter diagnostic(s) recorded; see `report.json`._\n",
            session.diagnostics.len()
        ));
    }
    out
}

/// Token accounting for the artifact header and `report.json`.
///
/// `masked` and `artifact` come from the caller: the renderer cannot compute
/// either. The masked count lives in the pipeline (it is the sum over every
/// row, and the artifact only carries the tail), and the artifact count is the
/// size of the text being rendered, which is why `Extraction::markdown`
/// measures a first pass and renders again with the answer. They used to be
/// hardcoded to 0, which made every artifact's header claim the session had no
/// masked tokens at all.
pub fn token_counts(artifact: &Artifact<'_>) -> TokenCounts {
    let raw: usize = artifact
        .session
        .source_paths
        .iter()
        .filter_map(|path| std::fs::metadata(path).ok())
        .map(|meta| meta.len() as usize / 4)
        .sum();
    TokenCounts {
        raw,
        masked: artifact.options.masked_tokens,
        artifact: artifact.options.artifact_tokens,
        tail: artifact.tail.iter().map(|row| row.tokens).sum(),
    }
}

fn one_line(text: &str, max_bytes: usize) -> String {
    // Collapse *runs* of whitespace, not just newlines. A stack trace and an
    // indented table both arrive with columns of spaces, and a quoted line that
    // keeps them reads as damage.
    let single = text.split_whitespace().collect::<Vec<&str>>().join(" ");
    crate::vendor::codex::truncate::truncate_middle_bytes(&strip_truncations(&single), max_bytes)
}

/// Remove truncation markers left by an earlier pass.
///
/// A ledger value was already shortened when it was recorded, so rendering it
/// again produced text like `…/scripts/…44 tokens truncated…/build.mjs`: a marker
/// that describes a truncation the reader cannot see, inside a line that was
/// then truncated again. Cut at the first marker instead and keep one ellipsis.
fn strip_truncations(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(open) = rest.find('\u{2026}') {
        let after = &rest[open + '\u{2026}'.len_utf8()..];
        // `…N tokens truncated…` or `…N bytes truncated…`
        let digits: String = after.chars().take_while(char::is_ascii_digit).collect();
        let marker = format!("{digits} tokens truncated\u{2026}");
        let marker_bytes = format!("{digits} bytes truncated\u{2026}");
        if after.starts_with(&marker) {
            out.push_str(&rest[..open]);
            out.push('\u{2026}');
            return out;
        }
        if after.starts_with(&marker_bytes) {
            out.push_str(&rest[..open]);
            out.push('\u{2026}');
            return out;
        }
        out.push_str(&rest[..open + '\u{2026}'.len_utf8()]);
        rest = after;
    }
    out.push_str(rest);
    out
}

/// The machine-readable artifact (`handoff.json`, schema `sctxx.handoff/v1`).
#[derive(Debug, Serialize)]
pub struct HandoffJson<'a> {
    pub schema: &'static str,
    pub sctxx: &'static str,
    pub session: SessionJson<'a>,
    pub mode: &'static str,
    pub llm: &'a str,
    pub prompts: Vec<(String, u32)>,
    pub items: Vec<&'a Item>,
    pub ledgers: &'a Ledgers,
    pub verification: &'a Reconciliation,
    pub tail: Vec<TailRowJson<'a>>,
}

/// Session identity, as it appears in `handoff.json`.
#[derive(Debug, Serialize)]
pub struct SessionJson<'a> {
    pub agent: &'static str,
    pub id: &'a str,
    pub source_paths: Vec<String>,
    pub source_hash: &'a str,
    pub events: usize,
    pub active: usize,
    pub user_turns: usize,
    #[serde(flatten)]
    pub meta: &'a crate::ir::SessionMeta,
}

/// One tail row in `handoff.json`.
#[derive(Debug, Serialize)]
pub struct TailRowJson<'a> {
    pub evt: u32,
    pub text: &'a str,
}

/// Render `handoff.json`.
pub fn json<'a>(artifact: &'a Artifact<'a>) -> HandoffJson<'a> {
    HandoffJson {
        schema: "sctxx.handoff/v1",
        sctxx: crate::VERSION,
        session: SessionJson {
            agent: artifact.session.agent.slug(),
            id: &artifact.session.id,
            source_paths: artifact
                .session
                .source_paths
                .iter()
                .map(|p| posix(p))
                .collect(),
            source_hash: &artifact.session.source_hash,
            events: artifact.session.events.len(),
            active: artifact.session.active.len(),
            user_turns: artifact.session.user_turns(),
            meta: &artifact.session.meta,
        },
        mode: artifact.options.mode,
        llm: &artifact.options.llm,
        prompts: crate::pipeline::fold::prompt::manifest(),
        items: artifact.state.active(),
        ledgers: artifact.ledgers,
        verification: artifact.reconciliation,
        tail: artifact
            .tail
            .iter()
            .map(|row| TailRowJson {
                evt: row.evt,
                text: &row.text,
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::{AgentKind, SessionMeta};
    use crate::pipeline::fold::ops::{Confidence, EvtRange, NewItem, Op};
    use crate::vendor::codex::tiered_input::Tier;

    fn session() -> Session {
        Session {
            agent: AgentKind::ClaudeCode,
            id: "7c1e8f82".into(),
            source_paths: vec![std::path::PathBuf::from(
                "/home/a/.claude/projects/p/7c1e8f82.jsonl",
            )],
            source_hash: "abcdef0123456789".into(),
            meta: SessionMeta {
                cwd: Some(std::path::PathBuf::from("/home/a/repo")),
                git_branch: Some("feat/x".into()),
                ..SessionMeta::default()
            },
            events: vec![],
            active: vec![],
            native_compactions: vec![],
            diagnostics: vec![],
        }
    }

    fn new_item(kind: ItemKind, text: &str) -> NewItem {
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

    fn artifact_with(state: &FoldState, ledgers: &Ledgers, tail: &[Row]) -> String {
        let session = session();
        let reconciliation = Reconciliation::default();
        let options = RenderOptions::default();
        markdown(&Artifact {
            session: &session,
            ledgers,
            state,
            reconciliation: &reconciliation,
            tail,
            options: &options,
        })
    }

    #[test]
    fn the_artifact_has_front_matter_a_preamble_and_every_layer() {
        let mut state = FoldState::new();
        state.apply(
            "c0",
            &Op::Add {
                item: new_item(ItemKind::Goal, "ship the migration"),
            },
        );
        let tail = vec![Row {
            evt: 99,
            tier: Tier::User,
            text: "[user] carry on".into(),
            tokens: 4,
            is_human_turn: true,
        }];
        let out = artifact_with(&state, &Ledgers::default(), &tail);
        assert!(out.starts_with("---\nschema: sctxx.handoff/v1"), "{out}");
        assert!(
            out.contains("A different coding agent worked on this task"),
            "{out}"
        );
        for heading in [
            "## L0 · Brief",
            "## L1 · Items",
            "## L2 · Recent activity",
            "## L3 · Retrieval",
        ] {
            assert!(out.contains(heading), "missing {heading}");
        }
        assert!(out.contains("[evt 10–20]"), "provenance missing: {out}");
    }

    #[test]
    fn a_constraint_is_quoted_in_the_brief() {
        let mut state = FoldState::new();
        let mut constraint = new_item(ItemKind::Constraint, "never push to main");
        constraint.quote = Some("never push to main without asking".into());
        state.apply("c0", &Op::Add { item: constraint });
        let out = artifact_with(&state, &Ledgers::default(), &[]);
        assert!(out.contains("Hard constraints"), "{out}");
        assert!(
            out.contains("\"never push to main without asking\""),
            "{out}"
        );
    }

    #[test]
    fn without_a_fold_the_goal_comes_from_the_first_user_message_and_says_so() {
        let mut ledgers = Ledgers::default();
        ledgers
            .user_messages
            .push(crate::pipeline::ledgers::UserMessageRecord {
                evt: 3,
                text: "implement the manifest loader with trust tiers".into(),
            });
        let out = artifact_with(&FoldState::new(), &ledgers, &[]);
        assert!(out.contains("not model-inferred"), "{out}");
        assert!(out.contains("manifest loader"), "{out}");
    }

    #[test]
    fn without_a_fold_the_next_actions_come_from_the_ledgers() {
        let mut ledgers = Ledgers::default();
        ledgers
            .commands
            .push(crate::pipeline::ledgers::CommandRecord {
                evt: 14,
                command: "pnpm test".into(),
                normalized: "pnpm test".into(),
                exit_code: Some(1),
                is_error: Some(true),
                category: crate::pipeline::ledgers::CmdCategory::Test,
                ..Default::default()
            });
        ledgers.plan = Some(crate::pipeline::ledgers::PlanLedger {
            evt: 5,
            items: vec![crate::ir::PlanItem {
                text: "wire tier 3".into(),
                status: "in_progress".into(),
            }],
        });
        let out = artifact_with(&FoldState::new(), &ledgers, &[]);
        assert!(
            out.contains("**Next actions** (derived from the ledgers"),
            "{out}"
        );
        assert!(out.contains("wire tier 3"), "{out}");
        assert!(out.contains("re-run `pnpm test`"), "{out}");
    }

    #[test]
    fn folded_next_actions_replace_the_ledger_derived_ones() {
        let mut state = FoldState::new();
        state.apply(
            "c0",
            &Op::Add {
                item: new_item(ItemKind::NextAction, "fix spawn()"),
            },
        );
        let mut ledgers = Ledgers::default();
        ledgers.plan = Some(crate::pipeline::ledgers::PlanLedger {
            evt: 5,
            items: vec![crate::ir::PlanItem {
                text: "wire tier 3".into(),
                status: "in_progress".into(),
            }],
        });
        let out = artifact_with(&state, &ledgers, &[]);
        assert!(out.contains("fix spawn()"), "{out}");
        assert!(!out.contains("derived from the ledgers"), "{out}");
    }

    #[test]
    fn the_last_user_request_is_surfaced_when_there_was_more_than_one() {
        let mut ledgers = Ledgers::default();
        for (evt, text) in [
            (0u32, "build the loader"),
            (12, "now make tier 3 sandboxed"),
        ] {
            ledgers
                .user_messages
                .push(crate::pipeline::ledgers::UserMessageRecord {
                    evt,
                    text: text.into(),
                });
        }
        let out = artifact_with(&FoldState::new(), &ledgers, &[]);
        assert!(
            out.contains("**Last user request**: now make tier 3 sandboxed"),
            "{out}"
        );
    }

    #[test]
    fn every_artifact_tells_the_next_agent_how_to_verify_and_how_to_expand() {
        let out = artifact_with(&FoldState::new(), &Ledgers::default(), &[]);
        assert!(out.contains("**Verify first**"), "{out}");
        assert!(out.contains("git status"), "{out}");
        assert!(out.contains("sctxx expand claude:7c1e8f82"), "{out}");
    }

    #[test]
    fn items_over_the_budget_are_named_rather_than_silently_dropped() {
        let mut state = FoldState::new();
        for i in 0..40 {
            state.apply(
                "c0",
                &Op::Add {
                    item: new_item(ItemKind::EnvFact, &format!("fact number {i} ").repeat(6)),
                },
            );
        }
        let session = session();
        let reconciliation = Reconciliation::default();
        let options = RenderOptions {
            budget: 300,
            ..RenderOptions::default()
        };
        let ledgers = Ledgers::default();
        let out = markdown(&Artifact {
            session: &session,
            ledgers: &ledgers,
            state: &state,
            reconciliation: &reconciliation,
            tail: &[],
            options: &options,
        });
        assert!(out.contains("omitted for the artifact budget"), "{out}");
    }

    #[test]
    fn a_contradiction_is_flagged_prominently() {
        let session = session();
        let reconciliation = Reconciliation {
            contradicted_files: vec!["src/gone.rs".into()],
            ..Reconciliation::default()
        };
        let options = RenderOptions::default();
        let ledgers = Ledgers::default();
        let out = markdown(&Artifact {
            session: &session,
            ledgers: &ledgers,
            state: &FoldState::new(),
            reconciliation: &reconciliation,
            tail: &[],
            options: &options,
        });
        assert!(out.contains("[!WARNING]"), "{out}");
        assert!(out.contains("src/gone.rs"), "{out}");
    }

    #[test]
    fn transcript_code_fences_cannot_escape_the_recency_tail() {
        // An assistant message containing a fenced block and a heading: both
        // must stay inside L2 instead of becoming artifact structure.
        let tail = vec![Row {
            evt: 99,
            tier: Tier::AssistantFinal,
            text: "[assistant] here:\n```sh\nnpm run dev\n```\n## What's now hot".into(),
            tokens: 20,
            is_human_turn: false,
        }];
        let out = artifact_with(&FoldState::new(), &Ledgers::default(), &tail);

        // Inside a fence, `## ...` is literal text, so the property to check
        // is that no transcript line escapes the fence and becomes structure.
        let mut inside_fence = false;
        let mut fence_marker = String::new();
        for line in out.lines() {
            if !inside_fence && line.starts_with("``") {
                inside_fence = true;
                // The marker is the leading backtick run; the rest is a
                // language tag (```text, ```sh).
                fence_marker = line.chars().take_while(|c| *c == '`').collect();
                continue;
            }
            if inside_fence {
                if line == fence_marker {
                    inside_fence = false;
                }
                continue;
            }
            assert!(
                !line.starts_with("## ")
                    || line.starts_with("## L0")
                    || line.starts_with("## L1")
                    || line.starts_with("## L2")
                    || line.starts_with("## L3"),
                "transcript content escaped the fence and became a heading: {line}"
            );
        }
        assert!(
            !inside_fence,
            "the recency tail fence was never closed:\n{out}"
        );
        assert!(
            out.contains("What's now hot"),
            "the tail content was lost:\n{out}"
        );
    }

    #[test]
    fn the_fence_is_always_longer_than_the_content_it_wraps() {
        assert_eq!(fence_for("no backticks"), "```");
        assert_eq!(fence_for("a ``` fence"), "````");
        assert_eq!(fence_for("a ````` fence"), "``````");
    }

    #[test]
    fn layers_can_be_selected() {
        let layers = Layers::parse("L0,L3").expect("parse");
        assert!(layers.brief && layers.retrieval);
        assert!(!layers.items && !layers.tail);
        assert_eq!(Layers::parse("L9").expect_err("reject").exit_code(), 2);
    }

    #[test]
    fn the_json_artifact_carries_the_schema_and_the_same_items() {
        let mut state = FoldState::new();
        state.apply(
            "c0",
            &Op::Add {
                item: new_item(ItemKind::Goal, "ship"),
            },
        );
        let session = session();
        let reconciliation = Reconciliation::default();
        let options = RenderOptions::default();
        let ledgers = Ledgers::default();
        let artifact = Artifact {
            session: &session,
            ledgers: &ledgers,
            state: &state,
            reconciliation: &reconciliation,
            tail: &[],
            options: &options,
        };
        let value = serde_json::to_value(json(&artifact)).expect("serialize");
        assert_eq!(value["schema"], "sctxx.handoff/v1");
        assert_eq!(value["items"][0]["id"], "G1");
        assert_eq!(value["session"]["agent"], "claude");
    }
}

#[cfg(test)]
mod semantic_render_tests {
    use super::*;
    use crate::pipeline::ledgers::Ledgers;
    use crate::pipeline::{SemanticState, fold::state::FoldState};

    fn artifact_with(semantic: SemanticState) -> String {
        let options = RenderOptions {
            semantic,
            ..RenderOptions::default()
        };
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
        let state = FoldState::new();
        let ledgers = Ledgers::default();
        let reconciliation = crate::pipeline::reconcile::Reconciliation::default();
        markdown(&Artifact {
            session: &session,
            ledgers: &ledgers,
            state: &state,
            reconciliation: &reconciliation,
            tail: &[],
            options: &options,
        })
    }

    #[test]
    fn an_empty_semantic_layer_is_stated_in_the_artifact_not_left_to_inference() {
        // The whole point: a reader must not have to notice that the sections a
        // standard handoff promises are missing.
        let unavailable = artifact_with(SemanticState::Unavailable);
        assert!(
            unavailable.contains("semantic: unavailable"),
            "the header must say it:\n{unavailable}"
        );
        assert!(
            unavailable.contains("UNAVAILABLE"),
            "and L0 must warn before anything is acted on:\n{unavailable}"
        );

        let degraded = artifact_with(SemanticState::Degraded);
        assert!(degraded.contains("semantic: degraded"));
        assert!(degraded.contains("EMPTY"));
    }

    #[test]
    fn the_ordinary_deterministic_artifact_carries_no_warning() {
        // Noise on the normal path is how warnings stop being read.
        let text = artifact_with(SemanticState::NotRequested);
        assert!(text.contains("semantic: not_requested"));
        assert!(!text.contains("UNAVAILABLE"));
        assert!(!text.contains("EMPTY"));
    }
}

#[cfg(test)]
mod end_state_render_tests {
    use super::*;
    use crate::pipeline::finalize::{DerivedStep, EndState, Finding, FindingKind, Resolution};
    use crate::pipeline::fold::ops::ItemKind;
    use crate::pipeline::ledgers::{Ledgers, UserMessageRecord};
    use crate::pipeline::{SemanticState, fold::state::FoldState};

    fn render_with(end_state: EndState, ledgers: Ledgers, budget: usize) -> String {
        let options = RenderOptions {
            semantic: SemanticState::NotRequested,
            end_state: Some(end_state),
            budget,
            ..RenderOptions::default()
        };
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
        let state = FoldState::new();
        let reconciliation = crate::pipeline::reconcile::Reconciliation::default();
        markdown(&Artifact {
            session: &session,
            ledgers: &ledgers,
            state: &state,
            reconciliation: &reconciliation,
            tail: &[],
            options: &options,
        })
    }

    fn ledgers_with_summary() -> Ledgers {
        Ledgers {
            user_messages: vec![UserMessageRecord {
                evt: 1,
                text: "onboard yourself to this project".into(),
            }],
            prior_summaries: vec![UserMessageRecord {
                evt: 900,
                text: "The session built a manifest loader and hit a sandbox error.".into(),
            }],
            // One file and one command, so the ledger the workset summarises is
            // actually there.
            files: vec![crate::pipeline::ledgers::FileRecord {
                path: "src/runtime.ts".into(),
                edits: 2,
                first_evt: 3,
                last_evt: 7,
                ..Default::default()
            }],
            commands: vec![crate::pipeline::ledgers::CommandRecord {
                evt: 14,
                command: "pnpm vitest run".into(),
                normalized: "pnpm vitest run".into(),
                exit_code: Some(1),
                is_error: Some(true),
                category: crate::pipeline::ledgers::CmdCategory::Test,
                output_head: String::new(),
                output_tail: String::new(),
            }],
            ..Default::default()
        }
    }

    #[test]
    fn findings_are_the_first_thing_in_the_brief() {
        let end = EndState {
            resolutions: Vec::new(),
            derived_step: None,
            findings: vec![Finding {
                kind: FindingKind::RepoMovedOn,
                text: "3 commit(s) landed after this session ended".into(),
            }],
        };
        let text = render_with(end, ledgers_with_summary(), 8_000);
        let brief = text.split("## L1").next().unwrap_or(&text);
        let heading = brief.find("**Before you act").expect("findings are in L0");
        let goal = brief.find("**Goal**").expect("the goal is in L0");
        assert!(
            heading < goal,
            "a reader must meet the contradiction before the goal:\n{brief}"
        );
        assert!(brief.contains("the repository moved on"), "{brief}");
    }

    #[test]
    fn a_derived_step_is_labelled_as_derived() {
        let end = EndState {
            resolutions: Vec::new(),
            findings: Vec::new(),
            derived_step: Some(DerivedStep {
                text: "derived from the ledger, not model-written: the last recorded command was \
                       `pnpm test` (FAILED, evt 14)"
                    .into(),
                evt: 14,
            }),
        };
        let text = render_with(end, ledgers_with_summary(), 8_000);
        assert!(
            text.contains("**Current step** — derived from the ledger"),
            "{text}"
        );
        assert!(text.contains("[evt 14]"), "{text}");
    }

    #[test]
    fn resolved_actions_are_listed_so_they_are_not_redone() {
        let end = EndState {
            resolutions: vec![Resolution {
                id: "N1".into(),
                text: "run `cargo test --lib`".into(),
                command: "cargo test --lib".into(),
                evt: 40,
            }],
            findings: Vec::new(),
            derived_step: None,
        };
        let text = render_with(end, ledgers_with_summary(), 8_000);
        assert!(text.contains("Already done"), "{text}");
        assert!(text.contains("cargo test --lib"), "{text}");
        assert!(text.contains("[evt 40]"), "{text}");
    }

    #[test]
    fn the_first_request_is_provenance_and_the_folded_goal_is_active() {
        // The review's point: on a ten-day session the opening prompt is where
        // the work started, not what it is now.
        let options = RenderOptions {
            budget: 8_000,
            ..RenderOptions::default()
        };
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
        let mut state = FoldState::new();
        state.apply(
            "c0",
            &crate::pipeline::fold::ops::Op::Add {
                item: crate::pipeline::fold::ops::NewItem {
                    kind: ItemKind::Goal,
                    text: "make tier 3 sandboxed, the manifest loader is done".into(),
                    why: None,
                    quote: None,
                    sources: vec![crate::pipeline::fold::ops::EvtRange::new(12, 12)],
                    rejected: Vec::new(),
                    confidence: crate::pipeline::fold::ops::Confidence::High,
                },
            },
        );
        let ledgers = ledgers_with_summary();
        let reconciliation = crate::pipeline::reconcile::Reconciliation::default();
        let text = markdown(&Artifact {
            session: &session,
            ledgers: &ledgers,
            state: &state,
            reconciliation: &reconciliation,
            tail: &[],
            options: &options,
        });
        let brief = text.split("## L1").next().unwrap_or(&text);
        assert!(
            brief.contains("**Original request** (where the work started)"),
            "{brief}"
        );
        assert!(brief.contains("**Active goal**"), "{brief}");
        assert!(brief.contains("make tier 3 sandboxed"), "{brief}");
    }

    #[test]
    fn a_provider_summary_is_surfaced_but_never_as_evidence() {
        let end = EndState::default();
        let text = render_with(end, ledgers_with_summary(), 8_000);
        let brief = text.split("## L1").next().unwrap_or(&text);
        assert!(brief.contains("**Provider summary**"), "{brief}");
        assert!(brief.contains("low trust"), "{brief}");
        assert!(brief.contains("not evidence"), "{brief}");
        assert!(brief.contains("[evt 900]"), "with its pointer: {brief}");
    }

    #[test]
    fn a_small_budget_gets_the_ledger_and_not_a_second_copy_of_it() {
        let end = EndState::default();
        let small = render_with(end.clone(), ledgers_with_summary(), 400);
        let large = render_with(end, ledgers_with_summary(), 8_000);
        assert!(!small.contains("### Active workset"), "{small}");
        assert!(large.contains("### Active workset"), "{large}");
    }

    #[test]
    fn the_workset_leads_the_ledger_it_summarises() {
        let end = EndState::default();
        let text = render_with(end, ledgers_with_summary(), 8_000);
        let workset = text.find("### Active workset").expect("a workset");
        let files = text.find("#### Files touched").expect("the full ledger");
        assert!(workset < files, "the summary leads the record:\n{text}");
    }
}
